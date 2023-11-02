//! Helper Functions for Flight Plans

use crate::grpc::client::GrpcClients;
use chrono::{DateTime, Utc};
use prost_wkt_types::Timestamp;
use std::collections::BinaryHeap;
use svc_storage_client_grpc::prelude::*;
use uuid::Uuid;

#[derive(Debug, Clone, Eq)]
pub struct FlightPlanSchedule {
    pub origin_vertiport_id: String,
    pub origin_vertipad_id: String,
    pub origin_timeslot_start: DateTime<Utc>,
    pub origin_timeslot_end: DateTime<Utc>,
    pub target_vertiport_id: String,
    pub target_vertipad_id: String,
    pub target_timeslot_start: DateTime<Utc>,
    pub target_timeslot_end: DateTime<Utc>,
    pub vehicle_id: String,
    pub draft: bool,
}

impl PartialEq for FlightPlanSchedule {
    fn eq(&self, other: &Self) -> bool {
        self.origin_timeslot_start == other.origin_timeslot_start
    }
}

impl PartialOrd for FlightPlanSchedule {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.origin_timeslot_start.cmp(&other.origin_timeslot_start))
    }
}

impl Ord for FlightPlanSchedule {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.origin_timeslot_start.cmp(&other.origin_timeslot_start)
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum FlightPlanError {
    ClientError,
    InvalidData,
}

impl std::fmt::Display for FlightPlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            FlightPlanError::ClientError => write!(f, "ClientError"),
            FlightPlanError::InvalidData => write!(f, "InvalidData"),
        }
    }
}

impl TryFrom<flight_plan::Object> for FlightPlanSchedule {
    type Error = FlightPlanError;

    fn try_from(flight_plan: flight_plan::Object) -> Result<Self, Self::Error> {
        let Some(data) = flight_plan.data else {
            router_error!("(try_from) Flight plan [{}] has no data.", flight_plan.id);
            return Err(FlightPlanError::InvalidData);
        };

        //
        // Must have valid origin and target times
        //
        let origin_timeslot_start = match data.origin_timeslot_start {
            Some(origin_timeslot_start) => origin_timeslot_start.into(),
            None => {
                router_error!(
                    "(try_from) Flight plan [{}] has no scheduled origin.",
                    flight_plan.id
                );
                return Err(FlightPlanError::InvalidData);
            }
        };
        let origin_timeslot_end = match data.origin_timeslot_end {
            Some(origin_timeslot_end) => origin_timeslot_end.into(),
            None => {
                router_error!(
                    "(try_from) Flight plan [{}] has no scheduled origin.",
                    flight_plan.id
                );
                return Err(FlightPlanError::InvalidData);
            }
        };

        let target_timeslot_start = match data.target_timeslot_start {
            Some(target_timeslot_start) => target_timeslot_start.into(),
            None => {
                router_error!(
                    "(try_from) Flight plan [{}] has no scheduled target.",
                    flight_plan.id
                );
                return Err(FlightPlanError::InvalidData);
            }
        };
        let target_timeslot_end = match data.target_timeslot_end {
            Some(target_timeslot_end) => target_timeslot_end.into(),
            None => {
                router_error!(
                    "(try_from) Flight plan [{}] has no scheduled target.",
                    flight_plan.id
                );
                return Err(FlightPlanError::InvalidData);
            }
        };

        //
        // Must have valid origin and target vertiports in UUID format
        //
        let Some(origin_vertiport_id) = data.origin_vertiport_id else {
            router_error!(
                "(try_from) Flight plan [{}] has no origin vertiport.",
                flight_plan.id
            );
            return Err(FlightPlanError::InvalidData);
        };

        let origin_vertiport_id = match Uuid::parse_str(&origin_vertiport_id) {
            Ok(id) => id.to_string(),
            Err(e) => {
                router_error!(
                    "(try_from) Flight plan [{}] has invalid origin vertiport id: {}",
                    flight_plan.id,
                    e
                );
                return Err(FlightPlanError::InvalidData);
            }
        };

        let Some(target_vertiport_id) = data.target_vertiport_id else {
            router_error!(
                "(try_from) Flight plan [{}] has no target vertiport.",
                flight_plan.id
            );
            return Err(FlightPlanError::InvalidData);
        };

        let target_vertiport_id = match Uuid::parse_str(&target_vertiport_id) {
            Ok(id) => id.to_string(),
            Err(e) => {
                router_error!(
                    "(try_from) Flight plan [{}] has invalid target vertiport id: {}",
                    flight_plan.id,
                    e
                );
                return Err(FlightPlanError::InvalidData);
            }
        };

        //
        // Must have a valid vehicle id in UUID format
        //
        let Ok(vehicle_id) = Uuid::parse_str(&data.vehicle_id) else {
            router_error!(
                "(try_from) Flight plan [{}] has no vehicle.",
                flight_plan.id
            );
            return Err(FlightPlanError::InvalidData);
        };

        Ok(FlightPlanSchedule {
            origin_vertiport_id,
            origin_vertipad_id: data.origin_vertipad_id,
            origin_timeslot_start,
            origin_timeslot_end,
            target_vertiport_id,
            target_vertipad_id: data.target_vertipad_id,
            target_timeslot_start,
            target_timeslot_end,
            vehicle_id: vehicle_id.to_string(),
            draft: false,
        })
    }
}

/// Gets all flight plans from storage in sorted order from
///  earliest to latest target time
pub async fn get_sorted_flight_plans(
    target_timeslot_end: &DateTime<Utc>,
    clients: &GrpcClients,
) -> Result<BinaryHeap<FlightPlanSchedule>, FlightPlanError> {
    let target_timeslot_end: Timestamp = (*target_timeslot_end).into();

    // TODO(R4): Further filter by vehicle type, etc.
    //  With hundreds of vehicles in the air, this will be a lot of data
    //   on each call.
    let mut filter = AdvancedSearchFilter::search_less_or_equal(
        "target_timeslot_end".to_owned(),
        target_timeslot_end.to_string(),
    )
    .and_is_null("deleted_at".to_owned())
    .and_not_in(
        "flight_status".to_owned(),
        vec![
            (flight_plan::FlightStatus::Finished as i32).to_string(),
            (flight_plan::FlightStatus::Cancelled as i32).to_string(),
        ],
    );

    filter.order_by = vec![
        SortOption {
            sort_field: "vehicle_id".to_string(),
            sort_order: SortOrder::Asc as i32,
        },
        SortOption {
            sort_field: "origin_timeslot_start".to_owned(),
            sort_order: SortOrder::Asc as i32,
        },
    ];

    let response = match clients.storage.flight_plan.search(filter).await {
        Ok(response) => response.into_inner(),
        Err(e) => {
            router_error!(
                "(get_sorted_flight_plans) Failed to get flight plans from storage: {}",
                e
            );
            return Err(FlightPlanError::ClientError);
        }
    };

    Ok(response
        .list
        .into_iter()
        .filter_map(|fp| FlightPlanSchedule::try_from(fp).ok())
        .collect::<BinaryHeap<FlightPlanSchedule>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flight_plan_schedule_try_from() {
        let expected_origin_vertiport_id = Uuid::new_v4().to_string();
        let expected_origin_vertipad_id = Uuid::new_v4().to_string();
        let expected_target_vertiport_id = Uuid::new_v4().to_string();
        let expected_target_vertipad_id = Uuid::new_v4().to_string();
        let expected_vehicle_id = Uuid::new_v4().to_string();

        let flight_plan = flight_plan::Object {
            id: Uuid::new_v4().to_string(),
            data: Some(flight_plan::Data {
                origin_vertiport_id: Some(expected_origin_vertiport_id.clone()),
                origin_vertipad_id: expected_origin_vertipad_id.clone(),
                target_vertiport_id: Some(expected_target_vertiport_id.clone()),
                target_vertipad_id: expected_target_vertipad_id.clone(),
                origin_timeslot_start: Some(Timestamp {
                    seconds: 0,
                    nanos: 0,
                }),
                origin_timeslot_end: Some(Timestamp {
                    seconds: 0,
                    nanos: 0,
                }),
                target_timeslot_start: Some(Timestamp {
                    seconds: 0,
                    nanos: 0,
                }),
                target_timeslot_end: Some(Timestamp {
                    seconds: 0,
                    nanos: 0,
                }),
                vehicle_id: expected_vehicle_id.clone(),
                ..Default::default()
            }),
            ..Default::default()
        };

        let flight_plan_schedule = FlightPlanSchedule::try_from(flight_plan).unwrap();
        assert_eq!(
            flight_plan_schedule.origin_vertiport_id,
            expected_origin_vertiport_id
        );
        assert_eq!(
            flight_plan_schedule.origin_vertipad_id,
            expected_origin_vertipad_id
        );
        assert_eq!(
            flight_plan_schedule.target_vertiport_id,
            expected_target_vertiport_id
        );
        assert_eq!(
            flight_plan_schedule.target_vertipad_id,
            expected_target_vertipad_id
        );
        assert_eq!(flight_plan_schedule.vehicle_id, expected_vehicle_id);
    }

    #[test]
    fn test_flight_plan_schedule_try_from_invalid_data() {
        let flight_plan = flight_plan::Object {
            id: "test".to_owned(),
            data: None,
            ..Default::default()
        };

        let e = FlightPlanSchedule::try_from(flight_plan).unwrap_err();
        assert_eq!(e, FlightPlanError::InvalidData);
    }
}
