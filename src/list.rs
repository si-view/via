use anyhow::Result;

use crate::cli::ListArgs;
use crate::process::{process_alive, Registry};

pub fn run(args: ListArgs) -> Result<()> {
    let mut registry = Registry::load()?;

    if registry.instances.is_empty() {
        println!("no managed instances");
        return Ok(());
    }

    // Collect and sort by name for stable output.
    let mut names: Vec<&str> = registry.instances.keys().map(|s| s.as_str()).collect();
    names.sort_unstable();

    // Column widths (minimum enforced).
    let name_w = names.iter().map(|n| n.len()).max().unwrap_or(4).max(4);
    let pid_w = 7;

    println!(
        "{:<name_w$}  {:<pid_w$}  {:<7}  {}",
        "NAME", "PID", "STATUS", "SOCK",
        name_w = name_w,
        pid_w = pid_w,
    );
    println!("{}", "-".repeat(name_w + pid_w + 7 + 50));

    let mut dead_names: Vec<String> = Vec::new();

    for name in &names {
        let inst = registry.instances.get(*name).unwrap();
        let alive = process_alive(inst.virtuoso_pid);
        let status = if alive { "running" } else { "dead   " };

        println!(
            "{:<name_w$}  {:<pid_w$}  {}  {}",
            inst.name,
            inst.virtuoso_pid,
            status,
            inst.sock.display(),
            name_w = name_w,
            pid_w = pid_w,
        );

        if (args.prune || args.dry_run) && !alive {
            dead_names.push(name.to_string());
        }
    }

    // Print detail lines (logs, workspace) below the table.
    println!();
    for name in &names {
        let inst = registry.instances.get(*name).unwrap();
        println!("  [{}]", inst.name);
        println!("    workspace    : {}", inst.workspace.display());
        println!("    virtuoso log : {}", inst.virtuoso_log.display());
        println!("    via log      : {}", inst.via_log.display());
        println!("    started      : {}", inst.started_at);
    }

    if !dead_names.is_empty() {
        if args.dry_run {
            println!(
                "\n[dry-run] would prune {} dead entr{}",
                dead_names.len(),
                if dead_names.len() == 1 { "y" } else { "ies" }
            );
            for name in &dead_names {
                println!("[dry-run]   - {name}");
            }
        } else if args.prune {
            for name in &dead_names {
                registry.instances.remove(name);
            }
            registry.save()?;
            println!("\npruned {} dead entr{}", dead_names.len(), if dead_names.len() == 1 { "y" } else { "ies" });
        }
    }

    Ok(())
}
