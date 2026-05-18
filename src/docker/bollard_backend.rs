use anyhow::{Context, Result};
use async_trait::async_trait;
use bollard::container::{
    Config as ContainerConfig, CreateContainerOptions, LogsOptions, RemoveContainerOptions,
    StartContainerOptions, StopContainerOptions,
};
use bollard::exec::{CreateExecOptions, StartExecOptions, StartExecResults};
use bollard::models::{HostConfig, Mount, MountTypeEnum, PortBinding};
use bollard::network::{CreateNetworkOptions, ListNetworksOptions, PruneNetworksOptions};
use bollard::Docker;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::process::Stdio;
use tokio::process::Command;

use super::*;

pub struct BollardBackend {
    docker: Docker,
}

impl BollardBackend {
    pub fn connect() -> Result<Self> {
        let docker = match resolve_context_host()? {
            Some(host) => connect_with_host(&host)?,
            None => Docker::connect_with_local_defaults().context("connect to docker daemon")?,
        };
        Ok(Self { docker })
    }
}

fn connect_with_host(host: &str) -> Result<Docker> {
    if let Some(path) = host.strip_prefix("unix://") {
        Docker::connect_with_unix(path, 120, bollard::API_DEFAULT_VERSION)
            .with_context(|| format!("connect unix:{path}"))
    } else if host.starts_with("tcp://")
        || host.starts_with("http://")
        || host.starts_with("https://")
    {
        Docker::connect_with_http(host, 120, bollard::API_DEFAULT_VERSION)
            .with_context(|| format!("connect {host}"))
    } else {
        anyhow::bail!("unsupported docker host scheme: {host}")
    }
}

fn resolve_context_host() -> Result<Option<String>> {
    if std::env::var_os("DOCKER_HOST").is_some() {
        return Ok(None);
    }
    let name = match std::env::var("DOCKER_CONTEXT") {
        Ok(n) => n,
        Err(_) => read_current_context()?.unwrap_or_else(|| "default".into()),
    };
    if name == "default" {
        return Ok(None);
    }
    read_context_host(&name)
}

fn read_current_context() -> Result<Option<String>> {
    let Some(home) = dirs::home_dir() else {
        return Ok(None);
    };
    let cfg_path = home.join(".docker/config.json");
    if !cfg_path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&cfg_path)?;
    let v: serde_json::Value = serde_json::from_str(&raw)?;
    Ok(v.get("currentContext")
        .and_then(|c| c.as_str())
        .map(String::from))
}

fn read_context_host(name: &str) -> Result<Option<String>> {
    use sha2::{Digest, Sha256};
    let Some(home) = dirs::home_dir() else {
        return Ok(None);
    };
    let digest = Sha256::digest(name.as_bytes());
    let id = hex::encode(digest);
    let meta_path = home.join(format!(".docker/contexts/meta/{id}/meta.json"));
    if !meta_path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&meta_path).with_context(|| format!("read {meta_path:?}"))?;
    let v: serde_json::Value = serde_json::from_str(&raw)?;
    let host = v
        .pointer("/Endpoints/docker/Host")
        .and_then(|h| h.as_str())
        .map(String::from);
    Ok(host)
}

#[async_trait]
impl Backend for BollardBackend {
    async fn container_get(&self, name: &str) -> Result<Option<String>> {
        match self.docker.inspect_container(name, None).await {
            Ok(c) => Ok(c.id),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn container_image_id(&self, name: &str) -> Result<Option<String>> {
        match self.docker.inspect_container(name, None).await {
            Ok(c) => Ok(c.image),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn image_get(&self, tag: &str) -> Result<Option<String>> {
        match self.docker.inspect_image(tag).await {
            Ok(i) => Ok(i.id),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn image_workdir(&self, tag: &str) -> Result<Option<String>> {
        match self.docker.inspect_image(tag).await {
            Ok(i) => Ok(i.config.and_then(|c| c.working_dir)),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn network_ensure(&self, name: &str) -> Result<()> {
        let mut filters = HashMap::new();
        filters.insert("name".to_string(), vec![name.to_string()]);
        let nets = self
            .docker
            .list_networks(Some(ListNetworksOptions { filters }))
            .await?;
        if nets.iter().any(|n| n.name.as_deref() == Some(name)) {
            return Ok(());
        }
        self.docker
            .create_network(CreateNetworkOptions {
                name: name.to_string(),
                driver: "bridge".to_string(),
                ..Default::default()
            })
            .await?;
        Ok(())
    }

    async fn network_prune(&self) -> Result<()> {
        self.docker
            .prune_networks(None::<PruneNetworksOptions<String>>)
            .await?;
        Ok(())
    }

    async fn run(&self, spec: RunSpec) -> Result<String> {
        self.network_ensure(&spec.network).await?;

        if self.image_get(&spec.image).await?.is_none() {
            tracing::info!("Pulling image: {}", spec.image);
            let opts = bollard::image::CreateImageOptions {
                from_image: spec.image.clone(),
                ..Default::default()
            };
            let mut stream = self.docker.create_image(Some(opts), None, None);
            while let Some(chunk) = stream.next().await {
                chunk.with_context(|| format!("pull {}", spec.image))?;
            }
        }

        let mut env: Vec<String> = spec
            .environment
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        env.sort();

        let mut port_bindings: HashMap<String, Option<Vec<PortBinding>>> = HashMap::new();
        let mut exposed: HashMap<String, HashMap<(), ()>> = HashMap::new();
        for (port, host) in &spec.ports {
            exposed.insert(port.clone(), HashMap::new());
            let binding = PortBinding {
                host_ip: None,
                host_port: if host.is_empty() {
                    None
                } else {
                    Some(host.clone())
                },
            };
            port_bindings.insert(port.clone(), Some(vec![binding]));
        }

        let mut mounts: Vec<Mount> = Vec::new();
        for v in &spec.volumes {
            mounts.push(Mount {
                target: Some(v.target.clone()),
                source: Some(v.source.to_string_lossy().into_owned()),
                typ: Some(MountTypeEnum::BIND),
                read_only: Some(v.mode == "ro"),
                ..Default::default()
            });
        }
        if spec.mount_cwd {
            let workdir = self
                .image_workdir(&spec.image)
                .await?
                .unwrap_or_else(|| "/app".to_string());
            mounts.push(Mount {
                target: Some(workdir),
                source: Some(std::env::current_dir()?.to_string_lossy().into_owned()),
                typ: Some(MountTypeEnum::BIND),
                read_only: Some(false),
                ..Default::default()
            });
        }

        let (entrypoint, cmd) = match &spec.command {
            Some(CommandSpec::Sleep) => (
                Some(vec!["/bin/sh".to_string()]),
                Some(vec![
                    "-c".to_string(),
                    "trap \"trap - TERM; kill -s TERM -- -$$\" TERM; tail -f /dev/null & wait"
                        .into(),
                ]),
            ),
            Some(CommandSpec::Custom(c)) => (None, Some(c.clone())),
            None => (None, None),
        };

        let host_config = HostConfig {
            mounts: Some(mounts),
            port_bindings: Some(port_bindings),
            network_mode: Some(spec.network.clone()),
            ..Default::default()
        };

        let cfg = ContainerConfig {
            image: Some(spec.image.clone()),
            cmd,
            entrypoint,
            env: Some(env),
            exposed_ports: Some(exposed),
            hostname: Some(spec.hostname.clone()),
            user: spec.user.clone(),
            host_config: Some(host_config),
            ..Default::default()
        };

        let created = self
            .docker
            .create_container(
                Some(CreateContainerOptions {
                    name: spec.name.clone(),
                    platform: None,
                }),
                cfg,
            )
            .await?;
        self.docker
            .start_container(&spec.name, None::<StartContainerOptions<String>>)
            .await?;
        Ok(created.id)
    }

    async fn start(&self, name: &str) -> Result<()> {
        match self
            .docker
            .start_container(name, None::<StartContainerOptions<String>>)
            .await
        {
            Ok(_) => Ok(()),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 304, ..
            }) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    async fn stop_remove(&self, name: &str) -> Result<()> {
        let _ = self
            .docker
            .stop_container(name, Some(StopContainerOptions { t: 10 }))
            .await;
        let _ = self
            .docker
            .remove_container(
                name,
                Some(RemoveContainerOptions {
                    v: true,
                    force: true,
                    ..Default::default()
                }),
            )
            .await;
        Ok(())
    }

    async fn status(&self, name: &str) -> Result<String> {
        match self.docker.inspect_container(name, None).await {
            Ok(c) => Ok(c
                .state
                .and_then(|s| s.status.map(|s| format!("{s:?}").to_lowercase()))
                .unwrap_or_else(|| "unknown".into())),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => Ok("notfound".into()),
            Err(e) => Err(e.into()),
        }
    }

    async fn logs(&self, name: &str) -> Result<String> {
        let mut stream = self.docker.logs(
            name,
            Some(LogsOptions::<String> {
                stdout: true,
                stderr: true,
                ..Default::default()
            }),
        );
        let mut out = String::new();
        while let Some(chunk) = stream.next().await {
            if let Ok(c) = chunk {
                out.push_str(&c.to_string());
            }
        }
        Ok(out)
    }

    async fn inspect_data(
        &self,
        name: &str,
        network: &str,
        inside: bool,
    ) -> Result<Option<ContainerData>> {
        let c = match self.docker.inspect_container(name, None).await {
            Ok(c) => c,
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let ns = c.network_settings.unwrap_or_default();
        let mut data = ContainerData::default();
        if inside {
            let nets = ns.networks.unwrap_or_default();
            data.host = nets.get(network).and_then(|n| n.ip_address.clone());
        } else {
            data.host = Some("localhost".into());
        }
        if let Some(ports) = ns.ports {
            for (port, bindings) in ports {
                if inside {
                    let bare = port.split('/').next().unwrap_or(&port).to_string();
                    data.ports.insert(format!("PORT;{port}"), bare);
                } else if let Some(bs) = bindings {
                    if let Some(b) = bs.first() {
                        if let Some(hp) = &b.host_port {
                            data.ports.insert(format!("PORT;{port}"), hp.clone());
                        }
                    }
                }
            }
        }
        Ok(Some(data))
    }

    async fn build(&self, opts: BuildOpts) -> Result<()> {
        // bollard's BuildKit + secret support is limited. Shell out to `docker build`
        // to match the Python tool's hybrid design (which itself shelled out for this).
        let dockerfile_path = format!("{}/{}", opts.directory.display(), opts.dockerfile);
        let mut cmd = Command::new("docker");
        cmd.arg("build")
            .arg(format!("--file={dockerfile_path}"))
            .arg(format!("--tag={}", opts.tag))
            .arg("--rm");
        if let Some(stage) = &opts.stage {
            cmd.arg(format!("--target={stage}"));
        }
        for (k, v) in &opts.buildargs {
            cmd.arg(format!("--build-arg={k}={v}"));
        }
        if opts.rebuild {
            cmd.arg("--no-cache").arg("--pull");
        }
        for (id, src) in &opts.secrets {
            let expanded = expand_tilde(src);
            cmd.arg(format!("--secret=id={id},source={}", expanded.display()));
        }
        cmd.arg(opts.directory.as_os_str());
        cmd.env("DOCKER_BUILDKIT", "1");
        cmd.stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        let status = cmd.status().await.context("spawn docker build")?;
        if !status.success() {
            anyhow::bail!("docker build failed: {status}");
        }
        Ok(())
    }

    async fn exec_run(&self, container: &str, command: &str, user: Option<&str>) -> Result<i64> {
        let exec = self
            .docker
            .create_exec(
                container,
                CreateExecOptions {
                    cmd: Some(vec!["sh".to_string(), "-c".into(), command.to_string()]),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    user: user.map(String::from),
                    ..Default::default()
                },
            )
            .await?;
        let started = self
            .docker
            .start_exec(&exec.id, None::<StartExecOptions>)
            .await?;
        if let StartExecResults::Attached { mut output, .. } = started {
            while let Some(chunk) = output.next().await {
                if let Ok(c) = chunk {
                    print!("{c}");
                }
            }
        }
        let inspect = self.docker.inspect_exec(&exec.id).await?;
        Ok(inspect.exit_code.unwrap_or(0))
    }

    async fn exec_interactive(
        &self,
        container: &str,
        command: &[String],
        user: Option<&str>,
    ) -> Result<()> {
        let mut cmd = Command::new("docker");
        cmd.arg("exec").arg("-ti");
        if let Some(u) = user {
            cmd.arg("-u").arg(u);
        }
        cmd.arg(container);
        if command.is_empty() {
            cmd.arg("bash");
        } else {
            for c in command {
                cmd.arg(c);
            }
        }
        let status = cmd.status().await.context("spawn docker exec")?;
        if !status.success() {
            anyhow::bail!("docker exec exited with {status}");
        }
        Ok(())
    }
}

fn expand_tilde(p: &std::path::Path) -> std::path::PathBuf {
    if let Ok(s) = p.strip_prefix("~") {
        if let Some(home) = dirs::home_dir() {
            return home.join(s);
        }
    }
    p.to_path_buf()
}
