// Copyright 2024 Golem Cloud
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

use crate::components::redis::Redis;
use crate::components::shard_manager::ShardManager;
use crate::components::template_service::TemplateService;
use crate::components::worker_executor::spawned::SpawnedWorkerExecutor;
use crate::components::worker_executor::WorkerExecutor;
use crate::components::worker_executor_cluster::WorkerExecutorCluster;
use crate::components::worker_service::WorkerService;
use async_trait::async_trait;
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tracing::{info, Level};

pub struct SpawnedWorkerExecutorCluster {
    worker_executors: Vec<Arc<dyn WorkerExecutor + Send + Sync + 'static>>,
    stopped_indices: Arc<Mutex<HashSet<usize>>>,
}

impl SpawnedWorkerExecutorCluster {
    pub async fn new(
        size: usize,
        base_http_port: u16,
        base_grpc_port: u16,
        executable: &Path,
        working_directory: &Path,
        redis: Arc<dyn Redis + Send + Sync + 'static>,
        template_service: Arc<dyn TemplateService + Send + Sync + 'static>,
        shard_manager: Arc<dyn ShardManager + Send + Sync + 'static>,
        worker_service: Arc<dyn WorkerService + Send + Sync + 'static>,
        verbosity: Level,
        out_level: Level,
        err_level: Level,
    ) -> Self {
        info!("Starting a cluster of golem-worker-executors of size {size}");
        let mut worker_executors = Vec::new();

        for i in 0..size {
            let http_port = base_http_port + i as u16;
            let grpc_port = base_grpc_port + i as u16;

            let worker_executor: Arc<dyn WorkerExecutor + Send + Sync + 'static> = Arc::new(
                SpawnedWorkerExecutor::new(
                    executable,
                    working_directory,
                    http_port,
                    grpc_port,
                    redis.clone(),
                    template_service.clone(),
                    shard_manager.clone(),
                    worker_service.clone(),
                    verbosity,
                    out_level,
                    err_level,
                )
                .await,
            );

            worker_executors.push(worker_executor);
        }
        Self {
            worker_executors,
            stopped_indices: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

#[async_trait]
impl WorkerExecutorCluster for SpawnedWorkerExecutorCluster {
    fn size(&self) -> usize {
        self.worker_executors.len()
    }

    fn kill_all(&self) {
        info!("Killing all worker executors");
        for worker_executor in &self.worker_executors {
            worker_executor.kill();
        }
    }

    async fn restart_all(&self) {
        info!("Restarting all worker executors");
        for worker_executor in &self.worker_executors {
            worker_executor.restart().await;
        }
    }

    fn stop(&self, index: usize) {
        let mut stopped = self.stopped_indices.lock().unwrap();
        if !stopped.contains(&index) {
            self.worker_executors[index].kill();
            stopped.insert(index);
        }
    }

    async fn start(&self, index: usize) {
        if self.stopped_indices().contains(&index) {
            self.worker_executors[index].restart().await;
            self.stopped_indices.lock().unwrap().remove(&index);
        }
    }

    fn to_vec(&self) -> Vec<Arc<dyn WorkerExecutor + Send + Sync + 'static>> {
        self.worker_executors.to_vec()
    }

    fn stopped_indices(&self) -> Vec<usize> {
        self.stopped_indices
            .lock()
            .unwrap()
            .iter()
            .copied()
            .collect()
    }

    fn started_indices(&self) -> Vec<usize> {
        let all_indices = HashSet::from_iter(0..self.worker_executors.len());
        let stopped_indices = self.stopped_indices.lock().unwrap();
        all_indices.difference(&stopped_indices).copied().collect()
    }
}

impl Drop for SpawnedWorkerExecutorCluster {
    fn drop(&mut self) {
        self.kill_all();
    }
}