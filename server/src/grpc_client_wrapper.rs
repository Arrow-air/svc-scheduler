use async_trait::async_trait;
use svc_storage_client_grpc::client::flight_plan_rpc_client::FlightPlanRpcClient;
use svc_storage_client_grpc::client::vehicle_rpc_client::VehicleRpcClient;
use svc_storage_client_grpc::client::vertipad_rpc_client::VertipadRpcClient;
use svc_storage_client_grpc::client::vertiport_rpc_client::VertiportRpcClient;
use svc_storage_client_grpc::client::{
    FlightPlan, FlightPlanData, FlightPlans, Id, SearchFilter, UpdateFlightPlan, Vehicles,
    Vertiport, Vertiports,
};
use tonic::transport::Channel;
use tonic::{Request, Response, Status};

fn err_msg() -> Status {
    Status::internal("Storage client not initialized")
}

#[derive(Debug)]
pub struct GRPCClients {
    pub flight_plan_client: FlightPlanRpcClient<Channel>,
    pub vertiport_client: VertiportRpcClient<Channel>,
    pub vertipad_client: VertipadRpcClient<Channel>,
    pub vehicle_client: VehicleRpcClient<Channel>,
}

#[async_trait]
pub trait StorageClientWrapperTrait {
    async fn vertiports(
        &self,
        request: Request<SearchFilter>,
    ) -> Result<Response<Vertiports>, Status>;
    async fn vertiport_by_id(&self, request: Request<Id>) -> Result<Response<Vertiport>, Status>;
    async fn flight_plan_by_id(&self, request: Request<Id>)
        -> Result<Response<FlightPlan>, Status>;
    async fn flight_plans(
        &self,
        request: Request<SearchFilter>,
    ) -> Result<Response<FlightPlans>, Status>;
    async fn insert_flight_plan(
        &self,
        request: Request<FlightPlanData>,
    ) -> Result<Response<FlightPlan>, Status>;
    async fn update_flight_plan(
        &self,
        request: Request<UpdateFlightPlan>,
    ) -> Result<Response<FlightPlan>, Status>;
    async fn vehicles(&self, request: Request<SearchFilter>) -> Result<Response<Vehicles>, Status>;
}

#[derive(Debug)]
pub struct StorageClientWrapper {
    pub grpc_clients: Option<GRPCClients>,
}

#[async_trait]
impl StorageClientWrapperTrait for StorageClientWrapper {
    async fn vertiports(
        &self,
        request: Request<SearchFilter>,
    ) -> Result<Response<Vertiports>, Status> {
        let mut vertiport_client = self
            .grpc_clients
            .as_ref()
            .ok_or_else(err_msg)
            .unwrap()
            .vertiport_client
            .clone();
        vertiport_client.vertiports(request).await
    }

    async fn vertiport_by_id(&self, request: Request<Id>) -> Result<Response<Vertiport>, Status> {
        let mut vertiport_client = self
            .grpc_clients
            .as_ref()
            .ok_or_else(err_msg)
            .unwrap()
            .vertiport_client
            .clone();
        vertiport_client.vertiport_by_id(request).await
    }

    async fn flight_plan_by_id(
        &self,
        request: Request<Id>,
    ) -> Result<Response<FlightPlan>, Status> {
        let mut fp_client = self
            .grpc_clients
            .as_ref()
            .ok_or_else(err_msg)
            .unwrap()
            .flight_plan_client
            .clone();
        fp_client.flight_plan_by_id(request).await
    }

    async fn flight_plans(
        &self,
        request: Request<SearchFilter>,
    ) -> Result<Response<FlightPlans>, Status> {
        let mut fp_client = self
            .grpc_clients
            .as_ref()
            .ok_or_else(err_msg)
            .unwrap()
            .flight_plan_client
            .clone();
        fp_client.flight_plans(request).await
    }

    async fn insert_flight_plan(
        &self,
        request: Request<FlightPlanData>,
    ) -> Result<Response<FlightPlan>, Status> {
        let mut fp_client = self
            .grpc_clients
            .as_ref()
            .ok_or_else(err_msg)
            .unwrap()
            .flight_plan_client
            .clone();
        fp_client.insert_flight_plan(request).await
    }

    async fn update_flight_plan(
        &self,
        request: Request<UpdateFlightPlan>,
    ) -> Result<Response<FlightPlan>, Status> {
        let mut fp_client = self
            .grpc_clients
            .as_ref()
            .ok_or_else(err_msg)
            .unwrap()
            .flight_plan_client
            .clone();
        fp_client.update_flight_plan(request).await
    }

    async fn vehicles(&self, request: Request<SearchFilter>) -> Result<Response<Vehicles>, Status> {
        let mut vehicle_client = self
            .grpc_clients
            .as_ref()
            .ok_or_else(err_msg)
            .unwrap()
            .vehicle_client
            .clone();
        vehicle_client.vehicles(request).await
    }
}
