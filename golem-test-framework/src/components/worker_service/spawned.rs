// Copyright 2024-2025 Golem Cloud
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::components::component_service::ComponentService;
use crate::components::rdb::Rdb;
use crate::components::shard_manager::ShardManager;
use crate::components::worker_service::{
    new_client, wait_for_startup, WorkerService, WorkerServiceEnvVars,
};
use crate::components::{ChildProcessLogger, GolemEnvVars};
use async_trait::async_trait;

use golem_api_grpc::proto::golem::worker::v1::worker_service_client::WorkerServiceClient;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tonic::transport::Channel;
use tracing::info;
use tracing::Level;

pub struct SpawnedWorkerService {
    http_port: u16,
    grpc_port: u16,
    custom_request_port: u16,
    child: Arc<Mutex<Option<Child>>>,
    _logger: ChildProcessLogger,
    client: Option<WorkerServiceClient<Channel>>,
}

impl SpawnedWorkerService {
    pub async fn new(
        executable: &Path,
        working_directory: &Path,
        http_port: u16,
        grpc_port: u16,
        custom_request_port: u16,
        component_service: Arc<dyn ComponentService + Send + Sync + 'static>,
        shard_manager: Arc<dyn ShardManager + Send + Sync + 'static>,
        rdb: Arc<dyn Rdb + Send + Sync + 'static>,
        verbosity: Level,
        out_level: Level,
        err_level: Level,
        shared_client: bool,
    ) -> Self {
        Self::new_base(
            Box::new(GolemEnvVars()),
            executable,
            working_directory,
            http_port,
            grpc_port,
            custom_request_port,
            component_service,
            shard_manager,
            rdb,
            verbosity,
            out_level,
            err_level,
            shared_client,
        )
        .await
    }

    pub async fn new_base(
        env_vars: Box<dyn WorkerServiceEnvVars + Send + Sync + 'static>,
        executable: &Path,
        working_directory: &Path,
        http_port: u16,
        grpc_port: u16,
        custom_request_port: u16,
        component_service: Arc<dyn ComponentService + Send + Sync + 'static>,
        shard_manager: Arc<dyn ShardManager + Send + Sync + 'static>,
        rdb: Arc<dyn Rdb + Send + Sync + 'static>,
        verbosity: Level,
        out_level: Level,
        err_level: Level,
        shared_client: bool,
    ) -> Self {
        info!("Starting golem-worker-service process");

        if !executable.exists() {
            panic!("Expected to have precompiled golem-worker-service at {executable:?}");
        }

        let mut child = Command::new(executable)
            .current_dir(working_directory)
            .envs(
                env_vars
                    .env_vars(
                        http_port,
                        grpc_port,
                        custom_request_port,
                        component_service,
                        shard_manager,
                        rdb,
                        verbosity,
                    )
                    .await,
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Failed to start golem-worker-service");

        let logger =
            ChildProcessLogger::log_child_process("[workersvc]", out_level, err_level, &mut child);

        wait_for_startup("localhost", grpc_port, Duration::from_secs(90)).await;

        Self {
            http_port,
            grpc_port,
            custom_request_port,
            child: Arc::new(Mutex::new(Some(child))),
            _logger: logger,
            client: if shared_client {
                Some(
                    new_client("localhost", grpc_port)
                        .await
                        .expect("Failed to create client"),
                )
            } else {
                None
            },
        }
    }

    fn blocking_kill(&self) {
        info!("Stopping golem-worker-service");
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.kill();
        }
    }
}

#[async_trait]
impl WorkerService for SpawnedWorkerService {
    async fn client(&self) -> crate::Result<WorkerServiceClient<Channel>> {
        match &self.client {
            Some(client) => Ok(client.clone()),
            None => Ok(new_client("localhost", self.grpc_port).await?),
        }
    }

    fn private_host(&self) -> String {
        "localhost".to_string()
    }

    fn private_http_port(&self) -> u16 {
        self.http_port
    }

    fn private_grpc_port(&self) -> u16 {
        self.grpc_port
    }

    fn private_custom_request_port(&self) -> u16 {
        self.custom_request_port
    }

    async fn kill(&self) {
        self.blocking_kill()
    }
}

impl Drop for SpawnedWorkerService {
    fn drop(&mut self) {
        self.blocking_kill()
    }
}
