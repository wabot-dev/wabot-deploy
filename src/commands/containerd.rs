//! `wabot-deploy containerd` — is the runtime reachable, and what does
//! it say?
//!
//! A diagnostic rather than a feature. The containerd client is written
//! against generated bindings with no high-level API, so the first
//! question — does the socket answer, is the namespace there — deserves
//! a command instead of being inferred from a deployment that failed.

use crate::runtime::client::{Containerd, RuncOptions};

pub async fn run(pull: Option<String>, run: Option<String>, port: u16) -> anyhow::Result<i32> {
    let client = match Containerd::connect().await {
        Ok(client) => client,
        Err(error) => {
            println!("containerd: {error}");
            println!();
            println!("  `wabot-deploy install` installs and starts it.");
            return Ok(1);
        }
    };

    println!("containerd");
    println!("  socket     {}", client.socket());
    println!("  version    {}", client.version().await?);

    client.ensure_namespace().await?;
    println!("  namespace  {} (ready)", crate::runtime::client::NAMESPACE);

    // The options every container this node creates will carry. Printed
    // because "which runtime is actually going to run" is the question
    // behind most of what goes wrong here.
    let options = RuncOptions::crun();
    println!();
    println!("runtime options, per container");
    println!("  shim       {}", crate::runtime::client::RUNTIME);
    println!("  binary     {}", options.binary_name);
    println!(
        "  cgroups    {}",
        if options.systemd_cgroup {
            "systemd"
        } else {
            "cgroupfs"
        }
    );

    if let Some(reference) = pull {
        println!();
        println!("image {reference}");
        let fetched = crate::runtime::images::ensure(&client, &reference, None).await?;
        println!(
            "  {}",
            if fetched {
                "pulled and unpacked"
            } else {
                "already here"
            }
        );

        // What the image says about itself, which is what a container
        // built from it has to be told.
        let config = crate::runtime::images::config(&client, &reference).await?;
        println!("  command    {:?}", config.command);
        println!("  env        {} variable(s)", config.env.len());
        println!(
            "  workdir    {}",
            config.working_dir.clone().unwrap_or_else(|| "/".into())
        );
        println!(
            "  user       {}",
            config.user.clone().unwrap_or_else(|| "root".into())
        );
        println!(
            "  ports      {}",
            if config.exposed_ports.is_empty() {
                "none declared".to_string()
            } else {
                config
                    .exposed_ports
                    .iter()
                    .map(|port| port.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        );
    }

    if let Some(reference) = run {
        println!();
        println!("running {reference}");

        // A fixed id, so a run left behind by a previous failure is
        // replaced rather than accumulating.
        const ID: &str = "wabot-deploy-check";
        let request = crate::runtime::spec::ContainerRequest {
            port: Some(port),
            ..Default::default()
        };

        let status =
            crate::runtime::containers::run(&client, ID, &reference, &request, None).await?;
        println!("  pid        {}", status.pid);
        println!("  status     {}", status.status);

        // A container that starts and exits immediately reports
        // RUNNING for a moment, so the useful question is whether it is
        // still there a beat later.
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        match crate::runtime::containers::status(&client, ID).await? {
            Some(after) if after.running() => {
                println!("  after 2s   still running");
            }
            Some(after) => println!("  after 2s   {} (exit {})", after.status, after.exit_code),
            None => println!("  after 2s   gone"),
        }

        // A diagnostic that left a container behind would be a
        // diagnostic nobody runs twice.
        crate::runtime::containers::remove(&client, ID).await?;
        println!("  cleaned up");
    }

    Ok(0)
}
