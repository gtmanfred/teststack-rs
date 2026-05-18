mod bollard_backend;
pub use bollard_backend::BollardBackend;

use anyhow::Result;
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct RunSpec {
    pub name: String,
    pub image: String,
    pub command: Option<CommandSpec>,
    pub environment: BTreeMap<String, String>,
    pub ports: BTreeMap<String, String>,
    pub volumes: Vec<VolumeMount>,
    pub network: String,
    pub hostname: String,
    pub mount_cwd: bool,
    pub user: Option<String>,
}

#[derive(Debug, Clone)]
pub enum CommandSpec {
    Sleep,
    Custom(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct VolumeMount {
    pub source: PathBuf,
    pub target: String,
    pub mode: String,
}

#[derive(Debug, Default, Clone)]
pub struct ContainerData {
    pub host: Option<String>,
    pub ports: BTreeMap<String, String>,
}

#[async_trait]
pub trait Backend: Send + Sync {
    async fn container_get(&self, name: &str) -> Result<Option<String>>;
    async fn container_image_id(&self, name: &str) -> Result<Option<String>>;
    async fn image_get(&self, tag: &str) -> Result<Option<String>>;
    async fn image_workdir(&self, tag: &str) -> Result<Option<String>>;
    async fn network_ensure(&self, name: &str) -> Result<()>;
    async fn network_prune(&self) -> Result<()>;
    async fn run(&self, spec: RunSpec) -> Result<String>;
    async fn start(&self, name: &str) -> Result<()>;
    async fn stop_remove(&self, name: &str) -> Result<()>;
    async fn status(&self, name: &str) -> Result<String>;
    async fn logs(&self, name: &str) -> Result<String>;
    async fn inspect_data(
        &self,
        name: &str,
        network: &str,
        inside: bool,
    ) -> Result<Option<ContainerData>>;
    async fn build(&self, opts: BuildOpts) -> Result<()>;
    async fn exec_run(&self, container: &str, command: &str, user: Option<&str>) -> Result<i64>;
    async fn exec_interactive(
        &self,
        container: &str,
        command: &[String],
        user: Option<&str>,
    ) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct BuildOpts {
    pub dockerfile: String,
    pub directory: PathBuf,
    pub tag: String,
    pub rebuild: bool,
    pub buildargs: BTreeMap<String, String>,
    pub secrets: BTreeMap<String, PathBuf>,
    pub stage: Option<String>,
}
