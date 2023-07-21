//! Stores the state of the router
use super::flightplan::create_flight_plan_data;
use crate::router::router_utils::schedule::Calendar;
use crate::{
    grpc::client::GrpcClients,
    router::router_types::{location::Location, node::Node},
};
use chrono::{DateTime, Duration, Utc};
use lib_common::time::datetime_to_timestamp;
use std::collections::HashMap;
use std::str::FromStr;
use svc_storage_client_grpc::prelude::*;
use tokio::sync::OnceCell;

use svc_gis_client_grpc::{service::Client, BestPathRequest, NodeType, DistanceTo};

const AVERAGE_AIRCRAFT_VELOCITY_M_PER_S: f32 = 20.0; // TODO(R4): Get from each vehicle model
const NEAREST_NEIGHBOR_MAX_RANGE_METERS: f64 = 100_000.0; // 100 KM
const NEAREST_NEIGHBOR_LIMIT: i32 = 10;

/// Query struct for generating nodes near a location.
#[derive(Debug, Copy, Clone)]
pub struct NearbyLocationQuery {
    ///location
    pub location: Location,
    ///radius
    pub radius: f32,
    ///capacity
    pub capacity: i32,
}

/// Query struct to find a route between two nodes
#[derive(Debug, Copy, Clone)]
pub struct RouteQuery {
    ///aircraft
    pub aircraft: Aircraft,
    ///from
    pub from: &'static Node,
    ///to
    pub to: &'static Node,
}

/// Enum with all Aircraft types
#[derive(Debug, Copy, Clone)]
pub enum Aircraft {
    ///Cargo aircraft
    Cargo,
}

static ARROW_CARGO_CONSTRAINT_METERS: f64 = 120000.0;

/// Time to block vertiport for cargo loading and takeoff
pub const LOADING_AND_TAKEOFF_TIME_MIN: f32 = 10.0;
/// Time to block vertiport for cargo unloading and landing
pub const LANDING_AND_UNLOADING_TIME_MIN: f32 = 10.0;
/// Average speed of cargo aircraft in kilometers per hour
pub const AVG_SPEED_KMH: f32 = 60.0;
/// Minimum time between suggested flight plans in case of multiple flights available
pub const FLIGHT_PLAN_GAP_MINUTES: f32 = 5.0;
/// Max amount of flight plans to return in case of large time window and multiple flights available
pub const MAX_RETURNED_FLIGHT_PLANS: i64 = 10;

/// Helper function to check if two time ranges overlap (touching ranges are not considered overlapping)
/// All parameters are in seconds since epoch
fn time_ranges_overlap(start1: i64, end1: i64, start2: i64, end2: i64) -> bool {
    start1 < end2 && start2 < end1
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum VehicleAvailability {
    Available,
    Unavailable,
    NoScheduleProvided,
    InvalidSchedule,
}

/// Checks if a vehicle is available for a given time window date_from to
///    date_from + flight_duration_minutes (this includes takeoff and landing time)
/// This checks both static schedule of the aircraft and existing flight plans which might overlap.
pub fn is_vehicle_available(
    vehicle: &vehicle::Object,
    date_from: DateTime<Utc>,
    flight_duration_minutes: i64,
    existing_flight_plans: &[flight_plan::Object],
) -> VehicleAvailability {
    let vehicle_id = &vehicle.id;
    
    let Some(vehicle_data) = vehicle.data.as_ref() else {
        router_error!(
            "(is_vehicle_available) Vehicle doesn't have data: {:?}",
            vehicle
        );
        return VehicleAvailability::Unavailable;
    }

    // TODO(R3): What's the default if a schedule isn't provided?
    let Some(vehicle_schedule) = vehicle_data.schedule.as_ref() else {
        return VehicleAvailability::NoScheduleProvided;
    };

    let vehicle_schedule = vehicle_schedule.as_str();
    let Ok(vehicle_schedule) = Calendar::from_str(vehicle_schedule) else {
        router_debug!(
            "(is_vehicle_available) Invalid schedule for vehicle {}: {}",
            vehicle_id,
            vehicle_schedule
        );

        return VehicleAvailability::InvalidSchedule;
    };

    let date_to = date_from + Duration::minutes(flight_duration_minutes);
    //check if vehicle is available as per schedule
    if !vehicle_schedule.is_available_between(
        date_from.with_timezone(&rrule::Tz::UTC),
        date_to.with_timezone(&rrule::Tz::UTC),
    ) {
        router_debug!("(is_vehicle_available) date_from [{}] - date_to [{}] don't fit in vehicle's schedule [{:?}].", date_from, date_to, vehicle_schedule);
        return VehicleAvailability::Unavailable;
    }

    //check if vehicle is available as per existing flight plans
    for existing_flight_plan in existing_flight_plans.iter() {
        if vehicle_id != &existing_flight_plan.id {
            continue;
        }

        let Some(data) = existing_flight_plan.data.as_ref() else {
            router_error!(
                "(is_vehicle_available) Existing flight plan doesn't have data: {:?}",
                existing_flight_plan
            );
            continue;
        };

        let Some(scheduled_arrival) = data.scheduled_arrival.as_ref() else {
            router_error!(
                "(is_vehicle_available) Existing flight plan doesn't have scheduled_arrival: {:?}",
                existing_flight_plan
            );
            continue;
        };

        let Some(scheduled_departure) = data.scheduled_departure.as_ref() else {
            router_error!(
                "(is_vehicle_available) Existing flight plan doesn't have scheduled_departure: {:?}",
                existing_flight_plan
            );
            continue;
        };

        if time_ranges_overlap(
            scheduled_departure.seconds,
            scheduled_arrival.seconds,
            date_from.timestamp(),
            date_to.timestamp(),
        ) {
            router_debug!("(is_vehicle_available) A flight is already scheduled with an overlapping time range for this vehicle [{}].", vehicle_id);
            return VehicleAvailability::Unavailable;
        }
    }

    VehicleAvailability::Available
}

/// Checks if vertiport is available for a given time window using the provided `at_date_time` value.
/// This checks both static schedule of vertiport and existing flight plans which might overlap.
/// `is_departure_vertiport` is used to determine if we are checking for departure or arrival vertiport.
///
/// ## Example scenario
/// flight_plan 1 has an `arrival_time` set at 2023-10-07T10:10 for vertipad 1
/// flight_plan 2 has an `departure_time` set at 2023-10-07T10:20 for vertipad 1
///
/// This results in the following schedule for vertipad 1:
/// ```ignore
/// 2023-10-07    |  10:00  |  10:05  |  10:10  |  10:15  |  10:20  |  10:25  |  10:30  |  10:35  |  10:40
/// ---------------------------------------------------------------------------------------------------------
///               |    landing and unloading
/// flight_plan 1 |    <------------------->
///               |                                              loading and takeoff
/// flight_plan 2 |                                             <------------------>
/// ---------------------------------------------------------------------------------------------------------
/// ```
/// With the above schedule, there is an available time slot for 10:10 - 10:20
/// For `is_departure_vertiport == true`
///   `at_date_time` = `2023-10-07T10:10` returns true
///   `at_date_time` = `2023-10-07T10:15` returns false
///   `at_date_time` = `2023-10-07T10:20` returns false
/// For `is_departure_vertiport == false`
///   `at_date_time` = `2023-10-07T10:10` returns false
///   `at_date_time` = `2023-10-07T10:15` returns false
///   `at_date_time` = `2023-10-07T10:20` returns true
pub fn is_vertiport_available(
    vertiport_id: String,
    vertiport_schedule: Option<String>,
    vertipads: &[vertipad::Object],
    at_date_time: DateTime<Utc>,
    existing_flight_plans: &[flight_plan::Object],
    is_departure_vertiport: bool,
) -> (bool, Vec<(String, i64)>) {
    let mut num_vertipads = vertipads.len();
    if num_vertipads == 0 {
        num_vertipads = 1
    };

    let vertiport_schedule = Calendar::from_str(vertiport_schedule.as_ref().unwrap().as_str()).unwrap();

    // Adjust availability times as per time window needed taking the
    // LOADING_AND_TAKEOFF_TIME_MIN into account for departure vertiports and
    // the LANDING_AND_UNLOADING_TIME_MIN for arrival vertiports
    let date_to;
    let date_from;
    if is_departure_vertiport {
        date_from = at_date_time;
        date_to = at_date_time + Duration::minutes(LOADING_AND_TAKEOFF_TIME_MIN as i64);
    } else {
        date_from = at_date_time - Duration::minutes(LANDING_AND_UNLOADING_TIME_MIN as i64);
        date_to = at_date_time;
    };
    //check if vertiport is available as per schedule
    if !vertiport_schedule.is_available_between(
        date_from.with_timezone(&rrule::Tz::UTC),
        date_to.with_timezone(&rrule::Tz::UTC),
    ) {
        router_debug!(
            "(is_vertiport_available) vertiport schedule does not match required times, returning."
        );
        return (false, vec![]);
    }

    // Adjust date_to and date_from to use for overlap search
    let date_to;
    let date_from;
    if is_departure_vertiport {
        date_from = at_date_time;
        date_to = at_date_time + Duration::minutes(LOADING_AND_TAKEOFF_TIME_MIN as i64);
    } else {
        date_from = at_date_time - Duration::minutes(LANDING_AND_UNLOADING_TIME_MIN as i64);
        date_to = at_date_time;
    };

    let conflicting_flight_plans_count = existing_flight_plans
        .iter()
        .filter(|flight_plan| {
            let flight_plan_data = flight_plan.data.as_ref().unwrap();
            if is_departure_vertiport {
                flight_plan_data.departure_vertiport_id.clone().unwrap() == vertiport_id
                    && flight_plan_data
                        .scheduled_departure
                        .as_ref()
                        .unwrap()
                        .seconds
                        + LOADING_AND_TAKEOFF_TIME_MIN as i64 * 60
                        > date_from.timestamp()
                    && flight_plan_data
                        .scheduled_departure
                        .as_ref()
                        .unwrap()
                        .seconds
                        < date_to.timestamp()
            } else {
                flight_plan_data.destination_vertiport_id.clone().unwrap() == vertiport_id
                    && flight_plan_data.scheduled_arrival.as_ref().unwrap().seconds
                        > date_from.timestamp()
                    && flight_plan_data.scheduled_arrival.as_ref().unwrap().seconds
                        - LANDING_AND_UNLOADING_TIME_MIN as i64 * 60
                        < date_to.timestamp()
            }
        })
        .count();
    let res = if num_vertipads > 1 {
        let vehicles_at_vertiport =
            get_all_vehicles_scheduled_for_vertiport(&vertiport_id, date_to, existing_flight_plans);
        (
            vehicles_at_vertiport.len() < num_vertipads,
            vehicles_at_vertiport,
        )
    } else {
        (conflicting_flight_plans_count == 0, vec![])
    };
    router_debug!(
        "(is_vertiport_available) Checking {} is departure: {}, is available for {} - {}? {}.",
        vertiport_id,
        is_departure_vertiport,
        date_from,
        date_to,
        res.0,
    );
    res
}

/// Finds all vehicles which are parked at or in flight to the vertiport at
/// specific timestamp.
/// Returns vector of tuples of (vehicle_id, minutes_to_arrival) where
/// minutes_to_arrival is 0 if vehicle is parked at the vertiport and up to 10
/// minutes if vehicle is landing.
pub fn get_all_vehicles_scheduled_for_vertiport(
    vertiport_id: &str,
    timestamp: DateTime<Utc>,
    existing_flight_plans: &[flight_plan::Object],
) -> Vec<(String, i64)> {
    let mut vehicles_plans_sorted: HashMap<String, Vec<flight_plan::Object>> = HashMap::new();
    existing_flight_plans
        .iter()
        .filter(|flight_plan| {
            let flight_plan_data = flight_plan.data.as_ref().unwrap();
            flight_plan_data.destination_vertiport_id.as_ref().unwrap() == vertiport_id
                && flight_plan_data
                    .scheduled_arrival
                    .as_ref()
                    .unwrap()
                    .seconds // arrival time needs to be less than 2x time needed - to allow landing and and then take off again)
                    < timestamp.timestamp() + LANDING_AND_UNLOADING_TIME_MIN as i64 * 60
        })
        .for_each(|flight_plan| {
            let vehicle_id = flight_plan.data.as_ref().unwrap().vehicle_id.clone();
            let entry = vehicles_plans_sorted.entry(vehicle_id).or_default();
            entry.push(flight_plan.clone());
        });

    //sort by scheduled arrival, latest first
    vehicles_plans_sorted
        .iter_mut()
        .for_each(|(_, flight_plans)| {
            flight_plans.sort_by(|a, b| {
                b.data
                    .as_ref()
                    .unwrap()
                    .scheduled_arrival
                    .as_ref()
                    .unwrap()
                    .seconds
                    .cmp(
                        &a.data
                            .as_ref()
                            .unwrap()
                            .scheduled_arrival
                            .as_ref()
                            .unwrap()
                            .seconds,
                    )
            });
        });

    //return only the latest flight plan for each vehicle
    let vehicles = vehicles_plans_sorted
        .iter()
        .map(|(vehicle_id, flight_plans)| {
            let mut minutes_to_arrival = (flight_plans
                .first()
                .unwrap()
                .data
                .as_ref()
                .unwrap()
                .scheduled_arrival
                .as_ref()
                .unwrap()
                .seconds
                - timestamp.timestamp())
                / 60;
            if minutes_to_arrival < 0 {
                minutes_to_arrival = 0;
            }
            (vehicle_id.clone(), minutes_to_arrival)
        })
        .collect();
    router_debug!(
        "(get_all_vehicles_scheduled_for_vertiport) Vehicles at vertiport: {} at a time: {} : {:?}.",
        &vertiport_id,
        timestamp,
        vehicles
    );
    vehicles
}

/// Gets vehicle location (vertiport_id) at given timestamp
/// Returns tuple of (vertiport_id, minutes_to_arrival)
/// If minutes_to_arrival is 0, vehicle is parked at the vertiport,
/// otherwise it is in flight to the vertiport and should arrive in minutes_to_arrival
pub fn get_vehicle_scheduled_location(
    vehicle: &vehicle::Object,
    timestamp: DateTime<Utc>,
    existing_flight_plans: &[flight_plan::Object],
) -> (String, i64) {
    let mut vehicle_flight_plans = existing_flight_plans
        .iter()
        .filter(|flight_plan| {
            flight_plan.data.as_ref().unwrap().vehicle_id == vehicle.id
                && flight_plan
                    .data
                    .as_ref()
                    .unwrap()
                    .scheduled_departure
                    .as_ref()
                    .unwrap()
                    .seconds
                    <= timestamp.timestamp()
        })
        .collect::<Vec<&flight_plan::Object>>();
    vehicle_flight_plans.sort_by(|a, b| {
        b.data
            .as_ref()
            .unwrap()
            .scheduled_departure
            .as_ref()
            .unwrap()
            .seconds
            .cmp(
                &a.data
                    .as_ref()
                    .unwrap()
                    .scheduled_departure
                    .as_ref()
                    .unwrap()
                    .seconds,
            )
    });

    router_debug!(
        "(get_vehicle_scheduled_location) Found flight plans for vehicle [{}]: {:?}",
        vehicle.id,
        vehicle_flight_plans
    );

    if vehicle_flight_plans.is_empty() {
        return (
            vehicle
                .data
                .as_ref()
                .unwrap()
                .last_vertiport_id
                .as_ref()
                .unwrap()
                .clone(),
            0,
        );
    }
    let vehicle_flight_plan = vehicle_flight_plans.first().unwrap();
    router_debug!(
        "(get_vehicle_scheduled_location) Vehicle {} had last flight plan {} with destination {}.",
        vehicle.id,
        vehicle_flight_plan.id.clone(),
        vehicle_flight_plan
            .data
            .as_ref()
            .unwrap()
            .destination_vertiport_id
            .as_ref()
            .unwrap()
    );
    let mut minutes_to_arrival = (vehicle_flight_plan
        .data
        .as_ref()
        .unwrap()
        .scheduled_arrival
        .as_ref()
        .unwrap()
        .seconds
        - timestamp.timestamp())
        / 60;
    if minutes_to_arrival < 0 {
        minutes_to_arrival = 0;
    }
    (
        vehicle_flight_plan
            .data
            .as_ref()
            .unwrap()
            .destination_vertiport_id
            .as_ref()
            .unwrap()
            .to_string(),
        minutes_to_arrival,
    )
}

/// Gets nearest gap for a reroute flight - takeoff and landing at the same vertiport
fn find_nearest_gap_for_reroute_flight(
    vertiport_id: String,
    vertiport_schedule: Option<String>,
    vertipads: &[vertipad::Object],
    date_from: DateTime<Utc>,
    vehicle_id: String,
    existing_flight_plans: &[flight_plan::Object],
) -> Option<DateTime<Utc>> {
    let mut time_from: Option<DateTime<Utc>> = None;
    for i in 0..6 {
        let added_time = date_from + Duration::minutes(i * LOADING_AND_TAKEOFF_TIME_MIN as i64);
        let (dep, vehicles_dep) = is_vertiport_available(
            vertiport_id.clone(),
            vertiport_schedule.clone(),
            vertipads,
            added_time,
            existing_flight_plans,
            true,
        );
        let (arr, vehicles_arr) = is_vertiport_available(
            vertiport_id.clone(),
            vertiport_schedule.clone(),
            vertipads,
            added_time + Duration::minutes(LANDING_AND_UNLOADING_TIME_MIN as i64),
            existing_flight_plans,
            false,
        );
        if (dep || vehicles_dep.contains(&(vehicle_id.clone(), 0)))
            && (arr || vehicles_arr.contains(&(vehicle_id.clone(), 0)))
        {
            time_from = Some(added_time);
            break;
        }
    }
    time_from
}

/// For the scenario where there is no available vehicle for the flight plan, this function find a deadhead flight plan
/// - summoning vehicle from the nearest vertiport to the departure vertiport so it can depart on time
/// Returns available vehicle and deadhead flight plan data if found, or (None, None) otherwise
#[allow(clippy::too_many_arguments)]
pub async fn find_deadhead_flight_plan(
    nearest_vertiports: &Vec<DistanceTo>,
    vehicles: &Vec<vehicle::Object>,
    vertiport_depart: &vertiport::Object,
    vertipads_depart: &[vertipad::Object],
    departure_time: DateTime<Utc>,
    existing_flight_plans: &[flight_plan::Object],
    block_aircraft_and_vertiports_minutes: i64,
    clients: &GrpcClients,
) -> (Option<vehicle::Object>, Option<flight_plan::Data>) {
    for &vertiport in nearest_vertiports {
        for vehicle in vehicles {

            // TODO(R4): Get this from vehicle model
            let vehicle_velocity_m_per_s: f32 = AVERAGE_AIRCRAFT_VELOCITY_M_PER_S;

            let duration_minutes: f32 = (vertiport.distance_meters/vehicle_velocity_m_per_s) * 60.;

            router_debug!(
                "(find_deadhead_flight_plan) Checking vehicle id:{} for departure time: {}",
                &vehicle.id,
                departure_time
            );

            let Some(vehicle_data) = vehicle.data.as_ref() else {
                router_error!(
                    "(find_deadhead_flight_plan) Vehicle [{}] has no data.",
                    &vehicle.id
                );

                continue;
            };

            let (vehicle_dest_vertiport, _minutes_to_arrival) = get_vehicle_scheduled_location(
                vehicle,
                departure_time - Duration::minutes(n_duration),
                existing_flight_plans,
            );
            if vehicle_dest_vertiport != *vertiport.uid {
                router_debug!(
                    "(find_deadhead_flight_plan) Vehicle [{}] not at or arriving to vertiport [{}].",
                    &vehicle.id,
                    vehicle_dest_vertiport
                );
                continue;
            }

            match is_vehicle_available(
                &vehicle.id,
                &vehicle_data,
                departure_time - Duration::minutes(n_duration),
                block_aircraft_and_vertiports_minutes,
                existing_flight_plans,
            ) {
                VehicleAvailability::Available => (),
                VehicleAvailability::Unavailable => {
                    router_debug!(
                        "(find_deadhead_flight_plan) Vehicle [{}] not available for departure time: {} and duration {} minutes.",
                        &vehicle.id, departure_time - Duration::minutes(n_duration), block_aircraft_and_vertiports_minutes
                    );
                    continue;
                }
                _ => {
                    router_debug!(
                        "(find_deadhead_flight_plan) Unable to determine vehicle availability: (id {})",
                        &vehicle.id
                    );
                    continue;
                }
            };

            let (is_departure_vertiport_available, _) = is_vertiport_available(
                vertiport.uid.clone(),
                vertiport.schedule.clone(),
                &[],
                departure_time - Duration::minutes(n_duration),
                existing_flight_plans,
                true,
            );
            let (is_arrival_vertiport_available, _) = is_vertiport_available(
                vertiport_depart.id.clone(),
                vertiport_depart.data.as_ref().unwrap().schedule.clone(),
                vertipads_depart,
                departure_time - Duration::minutes(LANDING_AND_UNLOADING_TIME_MIN as i64),
                existing_flight_plans,
                false,
            );

            router_debug!(
                "(find_deadhead_flight_plan) DEPARTURE TIME: {}, {}, {}.",
                departure_time,
                is_departure_vertiport_available,
                is_arrival_vertiport_available
            );
            
            if !is_departure_vertiport_available {
                router_debug!(
                    "(find_deadhead_flight_plan) Departure vertiport not available for departure time {}.",
                    departure_time - Duration::minutes(n_duration)
                );
                continue;
            }

            if !is_arrival_vertiport_available {
                router_debug!(
                    "(find_deadhead_flight_plan) Arrival vertiport not available for departure time {}.",
                    departure_time - Duration::minutes(LANDING_AND_UNLOADING_TIME_MIN as i64)
                );
                continue;
            }

            let Some(time_start) = datetime_to_timestamp(&(departure_time - Duration::minutes(n_duration))) else {
                router_debug!(
                    "(find_deadhead_flight_plan) Unable to convert departure time to timestamp: {}",
                    departure_time - Duration::minutes(n_duration)
                );
                continue;
            };

            let Some(time_end) = datetime_to_timestamp(&departure_time) else {
                router_debug!(
                    "(find_deadhead_flight_plan) Unable to convert departure time to timestamp: {}",
                    departure_time
                );
                continue;
            };

            let request = BestPathRequest {
                start_type: NodeType::Aircraft as i32,
                node_start_id: vehicle_data.registration_number.clone(),
                node_uuid_end: vertiport_depart.id.clone(),
                time_start: Some(time_start),
                time_end: Some(time_end),
            };

            let (path, _) = match gis_request(request, clients).await {
                Ok(response) => {
                    let path = response.0;
                    let cost = response.1;
                    (path, cost)
                }
                Err(e) => {
                    router_debug!(
                        "(find_deadhead_flight_plan) Error getting path from gis: {}",
                        e
                    );
                    continue;
                }
            };

            // add deadhead flight plan and return
            router_debug!(
                        "(find_deadhead_flight_plan) Found available vehicle [{}] from vertiport [{}], for a DH flight for a departure time {}.", vehicle.id, vertiport.uid.clone(),
                        departure_time - Duration::minutes(n_duration)
                    );
            return (
                Some(vehicle.clone()),
                Some(create_flight_plan_data(
                    vehicle.id.clone(),
                    vertiport.uid.clone(),
                    vertiport_depart.id.clone(),
                    departure_time - Duration::minutes(n_duration),
                    departure_time,
                    path,
                )),
            );
        }
    }
    (None, None)
}

/// In the scenario there is no vehicle available at the arrival vertiport, we can check
/// if there is availability at some other vertiport and re-route idle vehicle there.
/// This function finds such a flight plan and returns it
pub async fn find_rerouted_vehicle_flight_plan(
    vehicles_at_arrival_airport: &[(String, i64)],
    vertiport_arrive: &vertiport::Object,
    vertipads_arrive: &[vertipad::Object],
    arrival_time: &DateTime<Utc>,
    existing_flight_plans: &[flight_plan::Object],
    clients: &GrpcClients,
) -> Option<flight_plan::Data> {
    let found_vehicle = vehicles_at_arrival_airport
        .iter() //if there is a parked vehicle at the arrival vertiport, we can move it to some other vertiport
        .find(|(_, minutes_to_arrival)| *minutes_to_arrival == 0);
    found_vehicle?;
    router_debug!("(find_rerouted_vehicle_flight_plan) Checking if idle vehicle [{:#?}] from the arrival airport can be re-routed.", found_vehicle.unwrap());

    // TODO(R3) this should re-route the vehicle to the nearest vertiport or HUB, but
    // we don't have vertipads or HUB id in the graph to do this.
    // So we are just re-routing to the same vertiport in the future time instead
    let found_gap = find_nearest_gap_for_reroute_flight(
        vertiport_arrive.id.clone(),
        vertiport_arrive.data.as_ref().unwrap().schedule.clone(),
        vertipads_arrive,
        *arrival_time,
        found_vehicle.unwrap().0.clone(),
        existing_flight_plans,
    );

    let Some(found_gap) = found_gap else {
        router_debug!(
            "(find_rerouted_vehicle_flight_plan) No gap found for re-routing idle vehicle from the arrival vertiport."
        );
        return None;
    };

    router_debug!(
        "(find_rerouted_vehicle_flight_plan) Found a gap for re-routing idle vehicle from the arrival vertiport: {}",
        found_gap
    );

    let gap_time_start = found_gap;
    let gap_time_end = found_gap + Duration::minutes(LOADING_AND_TAKEOFF_TIME_MIN as i64);
    let Some(time_start) = datetime_to_timestamp(&gap_time_start) else {
        router_debug!(
            "(find_rerouted_vehicle_flight_plan) Error converting time_start datetime to timestamp."
        );
        return None;
    };

    let Some(time_end) = datetime_to_timestamp(&gap_time_end) else {
        router_debug!(
            "(find_rerouted_vehicle_flight_plan) Error converting time_end datetime to timestamp."
        );
        return None;
    };

    let request = BestPathRequest {
        start_type: NodeType::Aircraft as i32,
        node_start_id: found_vehicle.unwrap().0.clone(),
        node_uuid_end: vertiport_arrive.id.clone(),
        time_start: Some(time_start),
        time_end: Some(time_end),
    };

    let (path, _) = match gis_request(request, clients).await {
        Ok(response) => {
            let path = response.0;
            let cost = response.1;
            (path, cost)
        }
        Err(e) => {
            router_debug!(
                "(find_rerouted_vehicle_flight_plan) Error getting path from gis: {}",
                e
            );
            return None;
        }
    };

    Some(create_flight_plan_data(
        found_vehicle.unwrap().0.clone(),
        vertiport_arrive.id.clone(),
        vertiport_arrive.id.clone(),
        gap_time_start,
        gap_time_end,
        path,
    ))
}

/// Gets nearest vertiports to the requested vertiport
/// Returns tuple of:
///    sorted_vertiports_by_durations - vector of distances
pub async fn get_nearest_vertiports_vertiport_id(
    vertiport_depart: &vertiport::Object,
    clients: &GrpcClients
) -> Result<Vec<DistanceTo>, String> {
    router_debug!(
        "(nearest_vertiports) for departure vertiport {:?}",
        vertiport_depart
    );

    let request = tonic::Request::new(NearestNeighborRequest {
        start_node_id: vertiport_depart.id.clone(),
        start_type: NodeType::Vertiport as i32,
        end_type: NodeType::Vertiport as i32,
        limit: NEAREST_NEIGHBOR_LIMIT,
        max_range_meters: NEAREST_NEIGHBOR_MAX_RANGE_METERS,
    });

    let distances: Vec<DistanceTo> = match clients.gis.nearest_neighbor(request).await {
        Ok(response) => response.into_inner().distances,
        Err(error) => {
            router_error!(
                "(nearest_vertiports) Failed to get nearest vertiports: {}",
                error
            );
            return Err(error.to_string());
        }
    };

    Ok(distances)
}

async fn gis_request(
    request: BestPathRequest,
    clients: &GrpcClients,
) -> Result<(GeoLineString, f64), String> {
    let request = tonic::Request::new(request);
    let path = match clients.gis.best_path(request).await {
        Ok(response) => response.into_inner().segments,
        Err(error) => {
            router_error!("(get_possible_flights) Failed to get best path: {}", error);
            return Err(error.to_string());
        }
    };

    let (last_lat, last_lon) = match path.last() {
        Some(last) => (last.end_latitude, last.end_longitude),
        None => {
            router_error!("(get_possible_flights) No path found.");
            return Err("Path between vertiports not found".to_string());
        }
    };

    let cost = path.iter().map(|x| x.distance_meters as f64).sum();

    router_debug!("(get_possible_flights) Path: {:?}", path);
    router_debug!("(get_possible_flights) Cost: {:?}", cost);

    // convert segments to GeoLineString
    let mut points: Vec<GeoPoint> = path
        .iter()
        .map(|x| GeoPoint {
            latitude: x.start_latitude as f64,
            longitude: x.start_longitude as f64,
        })
        .collect();

    points.push(GeoPoint {
        latitude: last_lat as f64,
        longitude: last_lon as f64,
    });

    let path = GeoLineString { points };

    Ok((path, cost))
}

/// Creates all possible flight plans based on the given request
/// * `vertiport_depart` - Departure vertiport - svc-storage format
/// * `vertiport_arrive` - Arrival vertiport - svc-storage format
/// * `earliest_departure_time` - Earliest departure time of the time window
/// * `latest_arrival_time` - Latest arrival time of the time window
/// * `aircrafts` - Aircrafts serving the route and vertiports
/// # Returns
/// A vector of flight plans
#[allow(clippy::too_many_arguments)]
pub async fn get_possible_flights(
    vertiport_depart: vertiport::Object,
    vertiport_arrive: vertiport::Object,
    vertipads_depart: Vec<vertipad::Object>,
    vertipads_arrive: Vec<vertipad::Object>,
    earliest_departure_time: Option<Timestamp>,
    latest_arrival_time: Option<Timestamp>,
    vehicles: Vec<vehicle::Object>,
    existing_flight_plans: Vec<flight_plan::Object>,
    clients: &GrpcClients,
) -> Result<Vec<(flight_plan::Data, Vec<flight_plan::Data>)>, String> {
    router_info!("(get_possible_flights) Finding possible flights.");

    //1. Find route and cost between requested vertiports
    router_info!("(get_possible_flights) [1/5]: Finding route between vertiports.");
    let request = BestPathRequest {
        node_start_id: vertiport_depart.id.clone(),
        node_uuid_end: vertiport_arrive.id.clone(),
        start_type: NodeType::Vertiport as i32,
        time_start: match earliest_departure_time.clone() {
            Some(t) => Some(lib_common::time::Timestamp {
                seconds: t.seconds,
                nanos: t.nanos,
            }),
            None => None,
        },
        time_end: match latest_arrival_time.clone() {
            Some(t) => Some(lib_common::time::Timestamp {
                seconds: t.seconds,
                nanos: t.nanos,
            }),
            None => None,
        },
    };

    let (path, cost) = match gis_request(request, clients).await {
        Ok((path, cost)) => (path, cost),
        Err(e) => {
            router_error!("(get_possible_flights) Failed to get best path: {}", e);
            return Err(e);
        }
    };

    let earliest_departure_time: DateTime<Utc> = match earliest_departure_time {
        Some(timestamp) => timestamp.into(),
        None => {
            let error = "No earliest departure time given.";
            router_error!("(get_possible_flights) {}", error);
            return Err(String::from(error));
        }
    };
    
    let latest_arrival_time: DateTime<Utc> = match latest_arrival_time {
        Some(timestamp) => timestamp.into(),
        None => {
            let error = "No latest arrival time given.";
            router_error!("(get_possible_flights) {}", error);
            return Err(String::from(error));
        }
    };

    //1.1 Create a sorted vector of vertiports nearest to the departure and arrival vertiport (in case we need to create a deadhead flight)
    let nearest = nearest_vertiports(&vertiport_depart.id, clients).await?;
    router_info!(
        "(get_possible_flights) Found {} possible departure vertiports.",
        nearest.len()
    );

    //2. calculate blocking times for each vertiport and aircraft
    router_info!("(get_possible_flights) [2/5]: Calculating blocking times.");
    let block_aircraft_and_vertiports_minutes = estimate_flight_time_minutes(cost, Aircraft::Cargo);
    router_info!(
        "(get_possible_flights) Estimated flight time in minutes including takeoff and landing: {}",
        block_aircraft_and_vertiports_minutes
    );

    let time_window_duration_minutes: f32 =
        (latest_arrival_time - earliest_departure_time).num_minutes() as f32;
    router_debug!(
        "(get_possible_flights) Time window duration in minutes: {}",
        time_window_duration_minutes
    );
    if (time_window_duration_minutes - block_aircraft_and_vertiports_minutes) < 0.0 {
        router_info!("(get_possible_flights) Time window too small to schedule flight.");
        return Err("Time window too small to schedule flight.".to_string());
    }
    let mut num_flight_options: i64 = ((time_window_duration_minutes
        - block_aircraft_and_vertiports_minutes)
        / FLIGHT_PLAN_GAP_MINUTES)
        .floor() as i64
        + 1;
    if num_flight_options > MAX_RETURNED_FLIGHT_PLANS {
        num_flight_options = MAX_RETURNED_FLIGHT_PLANS;
    }
    //3. check vertiport schedules and flight plans
    router_info!(
        "(get_possible_flights) [3/5]: Checking vertiport schedules and flight plans for {} possible flight plans.",
        num_flight_options
    );
    let mut flight_plans: Vec<(flight_plan::Data, Vec<flight_plan::Data>)> = vec![];
    for i in 0..num_flight_options {
        let mut deadhead_flights: Vec<flight_plan::Data> = vec![];
        let mut departure_time: DateTime<Utc> = earliest_departure_time;
        departure_time += Duration::seconds(i * 60 * FLIGHT_PLAN_GAP_MINUTES as i64);
        let arrival_time =
            departure_time + Duration::minutes(block_aircraft_and_vertiports_minutes as i64);
        let (is_departure_vertiport_available, _) = is_vertiport_available(
            vertiport_depart.id.clone(),
            vertiport_depart
                .data
                .as_ref()
                .map_or(
                    Err(String::from(
                        "(get_possible_flights) No data provided for vertiport_depart.",
                    )),
                    Ok,
                )?
                .schedule
                .clone(),
            &vertipads_depart,
            departure_time,
            &existing_flight_plans,
            true,
        );
        let (is_arrival_vertiport_available, vehicles_at_arrival_airport) = is_vertiport_available(
            vertiport_arrive.id.clone(),
            vertiport_arrive
                .data
                .as_ref()
                .map_or(
                    Err(String::from(
                        "(get_possible_flights) No data provided for vertiport_arrive.",
                    )),
                    Ok,
                )?
                .schedule
                .clone(),
            &vertipads_arrive,
            arrival_time - Duration::minutes(LANDING_AND_UNLOADING_TIME_MIN as i64),
            &existing_flight_plans,
            false,
        );
        router_debug!(
            "(get_possible_flights) DEPARTURE TIME: {}, ARRIVAL TIME: {}, {}, {}.",
            departure_time,
            arrival_time,
            is_departure_vertiport_available,
            is_arrival_vertiport_available
        );
        if !is_departure_vertiport_available {
            router_debug!(
                "(get_possible_flights) Departure vertiport not available for departure time {}.",
                departure_time
            );
            continue;
        }
        if !is_arrival_vertiport_available {
            router_debug!(
                "(get_possible_flights) Arrival vertiport not available for departure time {}.",
                departure_time
            );

            let Some(flight_plan) = find_rerouted_vehicle_flight_plan(
                &vehicles_at_arrival_airport,
                &vertiport_arrive,
                &vertipads_arrive,
                &arrival_time,
                &existing_flight_plans,
                &clients
            ).await else {
                router_debug!("(get_possible_flights) No rerouted vehicle found.");
                continue;
            };

            deadhead_flights.push(flight_plan);
        }
        let mut available_vehicle: Option<vehicle::Object> = None;
        for vehicle in &vehicles {
            router_debug!(
                "(get_possible_flights) Checking vehicle id:{} for departure time: {}",
                &vehicle.id,
                departure_time
            );
            let (vehicle_vertiport_id, minutes_to_arrival) =
                get_vehicle_scheduled_location(vehicle, departure_time, &existing_flight_plans);
            if vehicle_vertiport_id != vertiport_depart.id || minutes_to_arrival > 0 {
                router_debug!(
                    "(get_possible_flights) Vehicle [{}] not available at location for requested time {}. It is/will be at vertiport [{}] in {} minutes.",
                    &vehicle.id, departure_time, vehicle_vertiport_id, minutes_to_arrival
                );
                continue;
            }

            let Some(vehicle_data) = vehicle.data.as_ref() else {
                router_debug!(
                    "(get_possible_flights) Vehicle [{}] has no data.",
                    &vehicle.id
                );
                continue;
            };

            match is_vehicle_available(
                &vehicle.id,
                &vehicle_data,
                departure_time,
                block_aircraft_and_vertiports_minutes as i64,
                &existing_flight_plans,
            ) {
                VehicleAvailability::Available => (),
                VehicleAvailability::Unavailable => {
                    router_debug!(
                        "(get_possible_flights) Vehicle [{}] not available for departure time: {} and duration {} minutes.",
                        &vehicle.id, departure_time, block_aircraft_and_vertiports_minutes
                    );
                    continue;
                }
                _ => {
                    router_debug!(
                        "(get_possible_flights) Unable to determine vehicle availability: (id {})",
                        &vehicle.id
                    );
                    continue;
                }
            };

            //when vehicle is available, break the "vehicles" loop early and add flight plan
            available_vehicle = Some(vehicle.clone());
            router_debug!("(get_possible_flights) Found available vehicle [{}] from vertiport [{}], for a flight for a departure time {}.", &vehicle.id, &vertiport_depart.id,
                        departure_time
                    );
            break;
        }
        // No simple flight plans found, looking for plans with deadhead flights
        if available_vehicle.is_none() {
            router_debug!(
                "(get_possible_flights) No available vehicles for departure time {}, looking for deadhead flights...",
                departure_time
            );

            let (a_vehicle, deadhead_flight_plan) = find_deadhead_flight_plan(
                &nearest,
                &vehicles,
                &vertiport_depart,
                &vertipads_depart,
                departure_time,
                &existing_flight_plans,
                block_aircraft_and_vertiports_minutes as i64,
                clients,
            )
            .await;

            if a_vehicle.is_some() {
                available_vehicle = a_vehicle;
                deadhead_flights.push(deadhead_flight_plan.unwrap());
            }
        }
        if available_vehicle.is_none() {
            router_debug!(
                "(get_possible_flights) DH: No available vehicles for departure time {} (including deadhead flights).",
                departure_time
            );
            continue;
        }
        //4. should check other constraints (cargo weight, number of passenger seats)
        //router_info!("(get_possible_flights) [4/5]: Checking other constraints (cargo weight, number of passenger seats)");
        flight_plans.push((
            create_flight_plan_data(
                available_vehicle.unwrap().id.clone(),
                vertiport_depart.id.clone(),
                vertiport_arrive.id.clone(),
                departure_time,
                arrival_time,
                path.clone(),
            ),
            deadhead_flights,
        ));
    }
    if flight_plans.is_empty() {
        let error = format!(
            "No flight plans found for given time window [{}] - [{}].",
            earliest_departure_time, latest_arrival_time
        );
        router_error!("(get_possible_flights) {}", error);
        return Err(error);
    }

    //5. return draft flight plan(s)
    router_info!(
        "(get_possible_flights) [5/5]: Returning {} draft flight plan(s).",
        flight_plans.len()
    );
    router_debug!("(get_possible_flights) Flight plans: {:?}", flight_plans);
    Ok(flight_plans)
}

/// Estimates the time needed to travel between two locations including loading and unloading
/// Estimate should be rather generous to block resources instead of potentially overloading them
pub fn estimate_flight_time_minutes(distance_meters: f64, aircraft: Aircraft) -> f32 {
    router_debug!("(estimate_flight_time_minutes) distance_meters: {}", distance_meters);
    router_debug!("(estimate_flight_time_minutes) aircraft: {:?}", aircraft);
    match aircraft {
        Aircraft::Cargo => {
            LOADING_AND_TAKEOFF_TIME_MIN
                + (distance_meters / 1000.0) as f32 / AVG_SPEED_KMH * 60.0
                + LANDING_AND_UNLOADING_TIME_MIN
        }
    }
}

// /// gets node by id
// pub async fn get_node_by_id(id: &str) -> Result<&'static Node, String> {
//     router_debug!("id: {}", id);
//     let nodes = get_nodes().await?;
//     let node = nodes
//         .iter()
//         .find(|node| node.uid == id)
//         .ok_or_else(|| "Node not found by id: ".to_owned() + id)?;
//     Ok(node)
// }

// /// Initialize the router with vertiports from the storage service
// pub async fn init_router_from_vertiports(vertiports: &[vertiport::Object]) -> Result<(), String> {
//     router_info!("(init_router_from_vertiports) Initializing router from vertiports.");
//     let mut nodes = vec![];
//     for vertiport in vertiports {
//         let data = match &vertiport.data {
//             Some(data) => data,
//             None => {
//                 return Err(format!(
//                     "(init_router_from_vertiports) No data provided for vertiport [{}].",
//                     vertiport.id
//                 ))
//             }
//         };
//         let geo_location = match &data.geo_location {
//             Some(polygon) => polygon,
//             None => {
//                 return Err(format!(
//                     "(init_router_from_vertiports) No geo_location provided for vertiport [{}].",
//                     vertiport.id
//                 ))
//             }
//         };
//         let latitude = OrderedFloat(geo_location.exterior.clone().ok_or(format!("(init_router_from_vertiports) No exterior points found for vertiport location of vertiport [{}]", vertiport.id))?.points[0].latitude as f32);
//         let longitude = OrderedFloat(geo_location.exterior.clone().ok_or(format!("(init_router_from_vertiports) No exterior points found for vertiport location of vertiport [{}]", vertiport.id))?.points[0].longitude as f32);
//         nodes.push(Node {
//             uid: vertiport.id.clone(),
//             location: Location {
//                 latitude,
//                 longitude,
//                 altitude_meters: OrderedFloat(0.0),
//             },
//             forward_to: None,
//             status: status::Status::Ok,
//             schedule: vertiport
//                 .data
//                 .as_ref()
//                 .ok_or_else(|| {
//                     format!(
//                         "Something went wrong when parsing schedule data of vertiport id: {}",
//                         vertiport.id
//                     )
//                 })
//                 .unwrap()
//                 .schedule
//                 .clone(),
//         })
//     }
//     set_nodes(nodes).await;
//     match get_router().await {
//         Ok(_) => Ok(()),
//         Err(e) => Err(e),
//     }
// }

// /// Checks if router is initialized
// pub fn is_router_initialized() -> bool {
//     ARROW_CARGO_ROUTER.get().is_some()
// }

// /// Get route
// pub async fn get_route(req: RouteQuery) -> Result<(Vec<Location>, f64), String> {
//     router_debug!("Getting route");
//     if !is_router_initialized() {
//         return Err("Arrow XL router not initialized. Try to initialize it first.".to_string());
//     }

//     let RouteQuery {
//         from,
//         to,
//         aircraft: _,
//     } = req;

//     let result = get_router()
//         .await?
//         .find_shortest_path(from, to, Algorithm::Dijkstra, None);

//     let Ok((cost, path)) = result else {
//         return Err(format!("{:?}", result.unwrap_err()));
//     };

//     router_debug!("cost: {}", cost);
//     router_debug!("path: {:?}", path);
//     let mut locations = vec![];
//     for node in path {
//         locations.push(
//             get_router()
//                 .await?
//                 .get_node_by_id(node)
//                 .ok_or(format!("Node not found by index {:?}", node))?
//                 .location,
//         );
//     }
//     router_debug!("locations: {:?}", locations);
//     router_info!("Finished getting route with cost: {}", cost);
//     Ok((locations, cost))
// }

// /// Gets the router
// /// Will initialize the router if it hasn't been set and if the NODES are available.
// /// Ensures initialization is done only once.
// pub(crate) async fn get_router() -> Result<&'static Router<'static>, String> {
//     if NODES.get().is_none() {
//         return Err("Nodes not initialized. Try to get some nodes first.".to_string());
//     }
//     ARROW_CARGO_ROUTER
//         .get_or_try_init(|| async move {
//             Ok(Router::new(
//                 get_nodes().await?,
//                 ARROW_CARGO_CONSTRAINT_METERS,
//                 |from, to| {
//                     let from_point: Point = from.as_node().location.into();
//                     let to_point: Point = to.as_node().location.into();
//                     from_point.geodesic_distance(&to_point)
//                 },
//                 |from, to| {
//                     let from_point: Point = from.as_node().location.into();
//                     let to_point: Point = to.as_node().location.into();
//                     from_point.geodesic_distance(&to_point)
//                 },
//             ))
//         })
//         .await
// }

// /// Gets nodes
// /// Returns error if nodes are not available yet
// pub(crate) async fn get_nodes() -> Result<&'static Vec<Node>, String> {
//     match NODES.get() {
//         Some(nodes) => Ok(nodes),
//         None => Err("Nodes not initialized. Try to get some nodes first.".to_string()),
//     }
// }
// /// Will initialize the nodes if it hasn't been initialized yet.
// /// Ensures initialization is done only once.
// pub(crate) async fn set_nodes(nodes: Vec<Node>) {
//     NODES.get_or_init(|| async move { nodes }).await;
// }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::*;
    use crate::{
        init_logger, router::router_types::location::Location,
        router::router_utils::mock::get_nearest_vertiports, test_util::ensure_storage_mock_data,
        Config,
    };
    use chrono::{TimeZone, Utc};
    use ordered_float::OrderedFloat;

    #[test]
    fn test_estimate_flight_time_minutes() {
        init_logger(&Config::try_from_env().unwrap_or_default());
        unit_test_info!("(test_estimate_flight_time_minutes) start");

        let distance_meters: f64 = (AVG_SPEED_KMH * 1000.0) as f64; // using AVG_SPEED_KMH since it's an easy calculation to make from there
        let aircraft = Aircraft::Cargo;
        let expected_time_minutes: f32 =
            LOADING_AND_TAKEOFF_TIME_MIN + 60.0 + LANDING_AND_UNLOADING_TIME_MIN; // If the distance is the same as the AVG_SPEED_KMH, it should take 60 minutes. Then we'll need to add the landing/ unloading time to get the expected minutes.

        let result = estimate_flight_time_minutes(distance_meters, aircraft);

        assert_eq!(result, expected_time_minutes);
        unit_test_info!("(test_estimate_flight_time_minutes) success");
    }

    #[tokio::test]
    async fn test_get_all_vehicles_scheduled_for_vertiport() {
        init_logger(&Config::try_from_env().unwrap_or_default());
        unit_test_info!("(get_all_vehicles_scheduled_for_vertiport) start");
        ensure_storage_mock_data().await;
        crate::grpc::queries::init_router().await;

        let latest_arrival_time: Timestamp = Utc
            .datetime_from_str("2022-10-27 15:00:00", "%Y-%m-%d %H:%M:%S")
            .unwrap()
            .into();
        let existing_flight_plans = crate::grpc::queries::query_flight_plans_for_latest_arrival(
            latest_arrival_time.clone(),
        )
        .await
        .unwrap();

        // The 3th vehicle we've inserted should be arriving at our 4th
        // vertiport at "2022-10-27 15:00:00".
        let vehicles = get_vehicles_from_storage().await;
        let expected_vehicle_id = &vehicles[2].id;
        let vertiports = get_vertiports_from_storage().await;
        let vertiport_id = &vertiports[3].id;
        let res = get_all_vehicles_scheduled_for_vertiport(
            vertiport_id,
            latest_arrival_time.into(),
            &existing_flight_plans,
        );
        unit_test_debug!(
            "(get_all_vehicles_scheduled_for_vertiport) Vehicles found: {:#?}",
            res
        );

        assert_eq!(res.len(), 1);
        assert_eq!(res[0], (expected_vehicle_id.clone(), 0));
        unit_test_info!("(get_all_vehicles_scheduled_for_vertiport) success");
    }

    #[test]
    fn test_is_vehicle_available_per_schedule_true() {
        init_logger(&Config::try_from_env().unwrap_or_default());
        unit_test_info!("(test_is_vehicle_available_per_schedule_true) start");

        // Construct a Vehicle Object using mock data and adding a known schedule.
        let vehicle_id = uuid::Uuid::new_v4();
        let vertiport_id = uuid::Uuid::new_v4();
        let schedule =
            "DTSTART:20221020T180000Z;DURATION:PT1H\nRRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR";
        let mut vehicle_data = vehicle::mock::get_data_obj();
        vehicle_data.last_vertiport_id = Some(vertiport_id.to_string());
        vehicle_data.schedule = Some(schedule.to_owned());
        let vehicle = vehicle::Object {
            id: vehicle_id.to_string(),
            data: Some(vehicle_data),
        };

        // Create a `date_from` value which should be within the vehicle's schedule
        let date_from: Timestamp = Utc
            .datetime_from_str("2022-10-26 15:00:00", "%Y-%m-%d %H:%M:%S")
            .unwrap()
            .into();
        let res =
            is_vehicle_available(vehicle_id.as_str(), &vehicle, date_from.into(), 60, &vec![]);

        assert!(res.is_ok());
        assert_eq!(res.unwrap(), true);
        unit_test_info!("(test_is_vehicle_available_per_schedule_true) success");
    }

    #[test]
    fn test_is_vehicle_available_per_schedule_false() {
        init_logger(&Config::try_from_env().unwrap_or_default());
        unit_test_info!("(test_is_vehicle_available_per_schedule_false) start");

        // Construct a Vehicle Object using mock data and adding a known schedule.
        let vehicle_id = uuid::Uuid::new_v4();
        let vertiport_id = uuid::Uuid::new_v4();
        let mut vehicle_data = vehicle::mock::get_data_obj();
        vehicle_data.last_vertiport_id = Some(vertiport_id.to_string());
        vehicle_data.schedule = Some(String::from(
            "DTSTART:20221020T180000Z;DURATION:PT1H\nRRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR",
        ));
        let vehicle = vehicle::Object {
            id: vehicle_id.to_string(),
            data: Some(vehicle_data),
        };

        // Create a `date_from` value which should be within the vehicle's schedule
        let date_from: Timestamp = Utc
            .datetime_from_str("2022-10-26 18:00:00", "%Y-%m-%d %H:%M:%S")
            .unwrap()
            .into();
        let res = is_vehicle_available(&vehicle_id, &vehicle, date_from.into(), 60, &vec![]);

        assert!(res.is_ok());
        assert_eq!(res.unwrap(), false);
        unit_test_info!("(test_is_vehicle_available_per_schedule_false) success");
    }

    #[tokio::test]
    async fn test_is_vehicle_available_true() {
        init_logger(&Config::try_from_env().unwrap_or_default());
        unit_test_info!("(test_is_vehicle_available_true) start");
        ensure_storage_mock_data().await;
        crate::grpc::queries::init_router().await;

        let latest_arrival_time: Timestamp = Utc
            .datetime_from_str("2022-10-26 15:00:00", "%Y-%m-%d %H:%M:%S")
            .unwrap()
            .into();
        let existing_flight_plans =
            crate::grpc::queries::query_flight_plans_for_latest_arrival(latest_arrival_time)
                .await
                .unwrap();
        let flight_plan = &existing_flight_plans[0];
        let flight_plan_data = flight_plan.data.as_ref().unwrap();

        // We'll pick a vehicle_id from the returned flight_plans, making sure
        // it's available
        let vehicle_id = flight_plan_data.vehicle_id.clone();
        let vertiport_id = flight_plan_data.departure_vertipad_id.clone();

        let mut vehicle_data = vehicle::mock::get_data_obj();
        vehicle_data.last_vertiport_id = Some(vertiport_id.to_string());
        vehicle_data.schedule = Some(String::from(
            "DTSTART:20221020T180000Z;DURATION:PT1H\nRRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR",
        ));
        let vehicle = vehicle::Object {
            id: vehicle_id.to_string(),
            data: Some(vehicle_data),
        };

        unit_test_debug!(
            "(test_is_vehicle_available_true) testing for vehicle: {:?}",
            vehicle.data
        );
        // This should be available
        let earliest_departure_time: Timestamp = Utc
            .datetime_from_str("2022-10-25 10:15:00", "%Y-%m-%d %H:%M:%S")
            .unwrap()
            .into();
        let res = is_vehicle_available(
            &vehicle_id,
            &vehicle,
            earliest_departure_time.into(),
            60,
            &existing_flight_plans,
        );

        assert!(res.is_ok());
        assert_eq!(res.unwrap(), true);
        unit_test_info!("(test_is_vehicle_available_true) success");
    }

    /// Takes vertiport 1 and gets all available flight_plans for the provided latest arrival.
    /// Then picks out the vehicle_id of the first flight_plan returned. Since
    /// this vehicle is already occupied for this flight_plan, the test should
    /// return false.
    #[tokio::test]
    async fn test_is_vehicle_available_false() {
        init_logger(&Config::try_from_env().unwrap_or_default());
        unit_test_info!("(test_is_vehicle_available_false) start");
        ensure_storage_mock_data().await;
        crate::grpc::queries::init_router().await;

        let latest_arrival_time: Timestamp = Utc
            .datetime_from_str("2022-10-25 15:00:00", "%Y-%m-%d %H:%M:%S")
            .unwrap()
            .into();

        let existing_flight_plans =
            crate::grpc::queries::query_flight_plans_for_latest_arrival(latest_arrival_time)
                .await
                .unwrap();
        let flight_plan = &existing_flight_plans[0];
        let flight_plan_data = flight_plan.data.as_ref().unwrap();

        // We'll pick a vehicle_id from the returned flight_plans, making sure
        // it's part of the test data set
        let vehicle_id = flight_plan_data.vehicle_id.clone();
        let vertiport_id = flight_plan_data.departure_vertipad_id.clone();

        let mut vehicle_data = vehicle::mock::get_data_obj();
        vehicle_data.last_vertiport_id = Some(vertiport_id.to_string());
        vehicle_data.schedule = Some(String::from(
            "DTSTART:20221020T180000Z;DURATION:PT1H\nRRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR",
        ));
        let vehicle = vehicle::Object {
            id: vehicle_id.to_string(),
            data: Some(vehicle_data),
        };

        // This should generate a conflict since we've already inserted a
        // flight_plan for this vehicle and this vertiport with a departure date
        // of "2022-10-25 14:20:00"
        let earliest_departure_time: Timestamp = Utc
            .datetime_from_str("2022-10-25 14:15:00", "%Y-%m-%d %H:%M:%S")
            .unwrap()
            .into();
        let res = is_vehicle_available(
            &vehicle_id,
            &vehicle,
            earliest_departure_time.into(),
            60,
            &existing_flight_plans,
        );

        assert!(res.is_ok());
        assert_eq!(res.unwrap(), false);
        unit_test_info!("(test_is_vehicle_available_false) success");
    }
}
