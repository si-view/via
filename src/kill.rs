use anyhow::{bail, Result};

use crate::cli::KillArgs;
use crate::process::{kill_process, process_alive, Registry};

pub fn run(args: KillArgs) -> Result<()> {
    let mut registry = Registry::load()?;

    let inst = registry
        .instances
        .get(&args.name)
        .ok_or_else(|| anyhow::anyhow!("no instance named '{}'", args.name))?
        .clone();

    if !process_alive(inst.virtuoso_pid) {
        eprintln!("warning: process {} is already dead; removing registry entry", inst.virtuoso_pid);
        registry.instances.remove(&args.name);
        registry.save()?;
        return Ok(());
    }

    if args.force {
        // SIGKILL
        let ret = unsafe { libc::kill(inst.virtuoso_pid as libc::pid_t, libc::SIGKILL) };
        if ret != 0 {
            bail!(
                "SIGKILL {} failed: {}",
                inst.virtuoso_pid,
                std::io::Error::last_os_error()
            );
        }
        println!("killed '{}' (pid {}, SIGKILL)", inst.name, inst.virtuoso_pid);
    } else {
        kill_process(inst.virtuoso_pid)?;
        println!("killed '{}' (pid {}, SIGTERM)", inst.name, inst.virtuoso_pid);
    }

    registry.instances.remove(&args.name);
    registry.save()?;
    Ok(())
}
