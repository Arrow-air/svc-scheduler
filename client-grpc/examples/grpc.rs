//! gRPC client implementation

use chrono::{offset::LocalResult::Single, TimeZone, Utc};
use lib_common::grpc::get_endpoint_from_env;
use svc_scheduler_client_grpc::prelude::{scheduler::*, *};

/// Example svc-scheduler-client-grpc
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (host, port) = get_endpoint_from_env("SERVER_HOSTNAME", "SERVER_PORT_GRPC");
    let client = SchedulerClient::new_client(&host, port, "scheduler");
    println!("Client created");
    println!(
        "NOTE: Ensure the server is running on {} or this example will fail.",
        client.get_address()
    );

    let ready = client.is_ready(ReadyRequest {}).await?.into_inner();
    assert_eq!(ready.ready, true);

    let departure_time = match Utc.with_ymd_and_hms(2022, 10, 25, 15, 0, 0) {
        Single(time) => time.timestamp(),
        _ => {
            println!("(main) failed to parse arrival time.");
            return Ok(());
        }
    };

    let arrival_time = match Utc.with_ymd_and_hms(2022, 10, 25, 16, 0, 0) {
        Single(time) => time.timestamp(),
        _ => {
            println!("(main) failed to parse arrival time.");
            return Ok(());
        }
    };

    let request = QueryFlightRequest {
        is_cargo: true,
        persons: Some(0),
        weight_grams: Some(5000),
        vertiport_depart_id: "a3509e85-6421-4dd1-8593-74950b88577e".to_string(),
        vertiport_arrive_id: "99c3bd83-79ef-4044-a903-5dac6f557193".to_string(),
        earliest_departure_time: Some(Timestamp {
            seconds: departure_time,
            nanos: 0,
        }),
        latest_arrival_time: Some(Timestamp {
            seconds: arrival_time,
            nanos: 0,
        }),
    };

    let response = client.query_flight(request).await?.into_inner().itineraries;
    let itinerary_id = (&response)[0].itinerary_id.clone();
    println!("(main) itinerary id={}", itinerary_id);

    let response = client
        .confirm_itinerary(ConfirmItineraryRequest {
            id: itinerary_id,
            user_id: "".to_string(),
        })
        .await?;

    println!("(main) RESPONSE={:?}", &response);

    Ok(())
}
