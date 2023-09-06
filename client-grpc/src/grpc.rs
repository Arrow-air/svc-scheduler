/// QueryFlightRequest
#[derive(Eq)]
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct QueryFlightRequest {
    /// is_cargo - true if cargo mission, false if people transport
    #[prost(bool, tag = "1")]
    pub is_cargo: bool,
    /// persons - number of people for transport
    #[prost(uint32, optional, tag = "2")]
    pub persons: ::core::option::Option<u32>,
    /// weight in grams
    #[prost(uint32, optional, tag = "3")]
    pub weight_grams: ::core::option::Option<u32>,
    /// requested earliest time of departure - beginning of the time window in which we search for a flight
    #[prost(message, optional, tag = "4")]
    pub earliest_departure_time: ::core::option::Option<::prost_wkt_types::Timestamp>,
    /// requested preferred time of arrival - end of the time window in which we search for a flight
    #[prost(message, optional, tag = "5")]
    pub latest_arrival_time: ::core::option::Option<::prost_wkt_types::Timestamp>,
    /// vertiport_depart_id
    #[prost(string, tag = "6")]
    pub vertiport_depart_id: ::prost::alloc::string::String,
    /// vertiport_depart_id
    #[prost(string, tag = "7")]
    pub vertiport_arrive_id: ::prost::alloc::string::String,
}
/// Confirms an itinerary by ID
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ConfirmItineraryRequest {
    /// The ID of the itinerary being confirmed
    #[prost(string, tag = "1")]
    pub id: ::prost::alloc::string::String,
    /// The ID of the user confirming the itinerary
    #[prost(string, tag = "2")]
    pub user_id: ::prost::alloc::string::String,
}
/// Itinerary includes id, flight plan and potential deadhead flights
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Itinerary {
    /// itinerary id
    #[prost(string, tag = "1")]
    pub itinerary_id: ::prost::alloc::string::String,
    /// flight_plan
    #[prost(message, repeated, tag = "2")]
    pub flight_plans: ::prost::alloc::vec::Vec<
        ::svc_storage_client_grpc::prelude::flight_plan::Object,
    >,
}
/// QueryFlightResponse
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct QueryFlightResponse {
    /// array/vector of itineraries items
    #[prost(message, repeated, tag = "1")]
    pub itineraries: ::prost::alloc::vec::Vec<Itinerary>,
}
/// ConfirmItineraryResponse
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ConfirmItineraryResponse {
    /// id of the itinerary
    #[prost(string, tag = "1")]
    pub id: ::prost::alloc::string::String,
    /// indicates if confirmation was successful
    #[prost(bool, tag = "2")]
    pub confirmed: bool,
    /// time when the flight was confirmed
    #[prost(message, optional, tag = "3")]
    pub confirmation_time: ::core::option::Option<::prost_wkt_types::Timestamp>,
}
/// CancelItineraryResponse
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct CancelItineraryResponse {
    /// id of the itinerary
    #[prost(string, tag = "1")]
    pub id: ::prost::alloc::string::String,
    /// indicates if cancellation was successful
    #[prost(bool, tag = "2")]
    pub cancelled: bool,
    /// time when the flight was cancelled
    #[prost(message, optional, tag = "3")]
    pub cancellation_time: ::core::option::Option<::prost_wkt_types::Timestamp>,
    /// reason of cancellation
    #[prost(string, tag = "4")]
    pub reason: ::prost::alloc::string::String,
}
/// Identification (typically UUID)
#[derive(Eq)]
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Id {
    /// The ID of the itinerary or flight plan
    #[prost(string, tag = "1")]
    pub id: ::prost::alloc::string::String,
}
/// Ready Request
///
/// No arguments
#[derive(Eq, Copy)]
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ReadyRequest {}
/// Ready Response
#[derive(Eq, Copy)]
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ReadyResponse {
    /// ready
    #[prost(bool, tag = "1")]
    pub ready: bool,
}
/// Generated client implementations.
#[cfg(not(tarpaulin_include))]
pub mod rpc_service_client {
    #![allow(unused_variables, dead_code, missing_docs, clippy::let_unit_value)]
    use tonic::codegen::*;
    use tonic::codegen::http::Uri;
    /// Scheduler service
    #[derive(Debug, Clone)]
    pub struct RpcServiceClient<T> {
        inner: tonic::client::Grpc<T>,
    }
    impl RpcServiceClient<tonic::transport::Channel> {
        /// Attempt to create a new client by connecting to a given endpoint.
        pub async fn connect<D>(dst: D) -> Result<Self, tonic::transport::Error>
        where
            D: std::convert::TryInto<tonic::transport::Endpoint>,
            D::Error: Into<StdError>,
        {
            let conn = tonic::transport::Endpoint::new(dst)?.connect().await?;
            Ok(Self::new(conn))
        }
    }
    impl<T> RpcServiceClient<T>
    where
        T: tonic::client::GrpcService<tonic::body::BoxBody>,
        T::Error: Into<StdError>,
        T::ResponseBody: Body<Data = Bytes> + Send + 'static,
        <T::ResponseBody as Body>::Error: Into<StdError> + Send,
    {
        pub fn new(inner: T) -> Self {
            let inner = tonic::client::Grpc::new(inner);
            Self { inner }
        }
        pub fn with_origin(inner: T, origin: Uri) -> Self {
            let inner = tonic::client::Grpc::with_origin(inner, origin);
            Self { inner }
        }
        pub fn with_interceptor<F>(
            inner: T,
            interceptor: F,
        ) -> RpcServiceClient<InterceptedService<T, F>>
        where
            F: tonic::service::Interceptor,
            T::ResponseBody: Default,
            T: tonic::codegen::Service<
                http::Request<tonic::body::BoxBody>,
                Response = http::Response<
                    <T as tonic::client::GrpcService<tonic::body::BoxBody>>::ResponseBody,
                >,
            >,
            <T as tonic::codegen::Service<
                http::Request<tonic::body::BoxBody>,
            >>::Error: Into<StdError> + Send + Sync,
        {
            RpcServiceClient::new(InterceptedService::new(inner, interceptor))
        }
        /// Compress requests with the given encoding.
        ///
        /// This requires the server to support it otherwise it might respond with an
        /// error.
        #[must_use]
        pub fn send_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.inner = self.inner.send_compressed(encoding);
            self
        }
        /// Enable decompressing responses.
        #[must_use]
        pub fn accept_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.inner = self.inner.accept_compressed(encoding);
            self
        }
        pub async fn query_flight(
            &mut self,
            request: impl tonic::IntoRequest<super::QueryFlightRequest>,
        ) -> Result<tonic::Response<super::QueryFlightResponse>, tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/grpc.RpcService/queryFlight",
            );
            self.inner.unary(request.into_request(), path, codec).await
        }
        pub async fn confirm_itinerary(
            &mut self,
            request: impl tonic::IntoRequest<super::ConfirmItineraryRequest>,
        ) -> Result<tonic::Response<super::ConfirmItineraryResponse>, tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/grpc.RpcService/confirmItinerary",
            );
            self.inner.unary(request.into_request(), path, codec).await
        }
        pub async fn cancel_itinerary(
            &mut self,
            request: impl tonic::IntoRequest<super::Id>,
        ) -> Result<tonic::Response<super::CancelItineraryResponse>, tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/grpc.RpcService/cancelItinerary",
            );
            self.inner.unary(request.into_request(), path, codec).await
        }
        pub async fn is_ready(
            &mut self,
            request: impl tonic::IntoRequest<super::ReadyRequest>,
        ) -> Result<tonic::Response<super::ReadyResponse>, tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::new(
                        tonic::Code::Unknown,
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static("/grpc.RpcService/isReady");
            self.inner.unary(request.into_request(), path, codec).await
        }
    }
}
