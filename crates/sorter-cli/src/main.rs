use anyhow::{anyhow, bail, Result};
use clap::{Args, Parser, Subcommand};
use sorter_core::{MatchSpec, Rule};
use sorter_fs::CollisionPolicy;
use sorter_runtime::{run_once, OneShotOptions, RunReport};
use sorter_storage::{
    Config, DirectoryConfig, ExecutionMode, HistoryStore, ScheduleTimezone, Settings,
    CURRENT_CONFIG_VERSION,
};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "sorter",
    version,
    about = "Safely organize files using deterministic rules"
)]
struct Cli {
    /// Increase structured log verbosity (-v, -vv, -vvv).
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Configuration file.
    #[arg(
        long,
        global = true,
        default_value = "sorter.toml",
        env = "SORTER_CONFIG"
    )]
    config: PathBuf,

    /// `SQLite` history database.
    #[arg(
        long,
        global = true,
        default_value = "sorter.db",
        env = "SORTER_DATABASE"
    )]
    database: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a documented starter configuration.
    Init {
        /// Replace an existing configuration.
        #[arg(long)]
        force: bool,
    },
    /// Validate the configuration without changing files.
    Validate,
    /// Scan and sort one configured or explicit directory.
    Run(RunArgs),
    /// Inspect or change rules.
    Rules {
        #[command(subcommand)]
        command: RuleCommand,
    },
    /// Inspect configured source directories.
    Directories {
        #[command(subcommand)]
        command: DirectoryCommand,
    },
    /// Show recent operation history.
    History {
        #[arg(long, default_value_t = 50)]
        limit: u32,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Args)]
struct RunArgs {
    /// Source path. Uses all configured rules; mutually exclusive with --directory.
    path: Option<PathBuf>,
    /// Configured directory id.
    #[arg(long, conflicts_with = "path")]
    directory: Option<String>,
    /// Show the plan without changing files or history.
    #[arg(long)]
    dry_run: bool,
    /// Recurse into subdirectories (explicit paths only).
    #[arg(long)]
    recursive: bool,
    /// Emit one JSON document.
    #[arg(long)]
    json: bool,
    /// Override the configured collision policy.
    #[arg(long, value_enum)]
    collision: Option<CliCollisionPolicy>,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum CliCollisionPolicy {
    Skip,
    RenameWithSuffix,
    Error,
}

impl From<CliCollisionPolicy> for CollisionPolicy {
    fn from(value: CliCollisionPolicy) -> Self {
        match value {
            CliCollisionPolicy::Skip => Self::Skip,
            CliCollisionPolicy::RenameWithSuffix => Self::RenameWithSuffix,
            CliCollisionPolicy::Error => Self::Error,
        }
    }
}

#[derive(Debug, Subcommand)]
enum RuleCommand {
    List {
        #[arg(long)]
        json: bool,
    },
    Enable {
        id: String,
    },
    Disable {
        id: String,
    },
}

#[derive(Debug, Subcommand)]
enum DirectoryCommand {
    List {
        #[arg(long)]
        json: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging(cli.verbose);
    match execute(cli) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::from(1)
        }
    }
}

fn init_logging(verbosity: u8) {
    let default_level = match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .compact()
        .init();
}

fn execute(cli: Cli) -> Result<ExitCode> {
    match cli.command {
        Command::Init { force } => init_config(&cli.config, force),
        Command::Validate => {
            Config::load(&cli.config)?;
            println!("configuration is valid: {}", cli.config.display());
            Ok(ExitCode::SUCCESS)
        }
        Command::Run(args) => run_command(&cli.config, &cli.database, &args),
        Command::Rules { command } => rules_command(&cli.config, command),
        Command::Directories { command } => directories_command(&cli.config, &command),
        Command::History { limit, json } => history_command(&cli.database, limit, json),
    }
}

fn init_config(path: &Path, force: bool) -> Result<ExitCode> {
    if path.exists() && !force {
        bail!(
            "configuration already exists at {}; pass --force to replace it",
            path.display()
        );
    }
    starter_config().save(path)?;
    println!("created configuration: {}", path.display());
    Ok(ExitCode::SUCCESS)
}

fn run_command(config_path: &Path, database_path: &Path, args: &RunArgs) -> Result<ExitCode> {
    let config = Config::load(config_path)?;
    let collision_policy = args
        .collision
        .map_or(config.settings.collision_policy, Into::into);

    let (source, recursive, rules) = select_run(&config, args)?;
    let history = if args.dry_run {
        None
    } else {
        Some(HistoryStore::open(database_path)?)
    };
    let report = run_once(
        &OneShotOptions {
            source: &source,
            recursive,
            dry_run: args.dry_run,
            collision_policy,
            rules: &rules,
        },
        history.as_ref(),
    )?;
    print_run_report(&report, args.json)?;
    if report.has_failures() {
        Ok(ExitCode::from(3))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn select_run(config: &Config, args: &RunArgs) -> Result<(PathBuf, bool, Vec<Rule>)> {
    if let Some(path) = &args.path {
        return Ok((path.clone(), args.recursive, config.rules.clone()));
    }

    let directory = if let Some(id) = &args.directory {
        config
            .directories
            .iter()
            .find(|directory| directory.id == *id)
            .ok_or_else(|| anyhow!("unknown directory id: {id}"))?
    } else {
        let enabled: Vec<_> = config
            .directories
            .iter()
            .filter(|directory| directory.enabled)
            .collect();
        if enabled.len() != 1 {
            bail!(
                "select a source path or --directory <id>; {} enabled directories are configured",
                enabled.len()
            );
        }
        enabled[0]
    };
    if !directory.enabled {
        bail!("directory {} is disabled", directory.id);
    }
    Ok((
        directory.path.clone(),
        directory.recursive,
        config.rules_for(directory),
    ))
}

fn print_run_report(report: &RunReport, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }
    for item in &report.items {
        let action = match item.status {
            sorter_runtime::ItemStatus::Planned => "PLAN",
            sorter_runtime::ItemStatus::Moved => "MOVED",
            sorter_runtime::ItemStatus::SkippedCollision => "SKIP",
            sorter_runtime::ItemStatus::Failed => "FAILED",
        };
        println!(
            "{action:6} {} -> {} [{}]{}",
            item.source.display(),
            item.destination.display(),
            item.rule_id,
            item.error
                .as_ref()
                .map_or_else(String::new, |message| format!(" {message}"))
        );
    }
    for scan_error in &report.scan_errors {
        eprintln!("SCAN ERROR {scan_error}");
    }
    println!(
        "scanned={} matched={} moved={} skipped={} failed={} dry_run={}",
        report.scanned, report.matched, report.moved, report.skipped, report.failed, report.dry_run
    );
    Ok(())
}

fn rules_command(path: &Path, command: RuleCommand) -> Result<ExitCode> {
    let mut config = Config::load(path)?;
    match command {
        RuleCommand::List { json } => {
            if json {
                println!("{}", serde_json::to_string_pretty(&config.rules)?);
            } else {
                for rule in &config.rules {
                    println!(
                        "{}\t{}\tpriority={}\t{}",
                        rule.id,
                        if rule.enabled { "enabled" } else { "disabled" },
                        rule.priority,
                        rule.destination.display()
                    );
                }
            }
        }
        RuleCommand::Enable { id } => set_rule_enabled(&mut config, path, &id, true)?,
        RuleCommand::Disable { id } => set_rule_enabled(&mut config, path, &id, false)?,
    }
    Ok(ExitCode::SUCCESS)
}

fn set_rule_enabled(config: &mut Config, path: &Path, id: &str, enabled: bool) -> Result<()> {
    let rule = config
        .rules
        .iter_mut()
        .find(|rule| rule.id == id)
        .ok_or_else(|| anyhow!("unknown rule id: {id}"))?;
    rule.enabled = enabled;
    config.save(path)?;
    println!("rule {id} {}", if enabled { "enabled" } else { "disabled" });
    Ok(())
}

fn directories_command(path: &Path, command: &DirectoryCommand) -> Result<ExitCode> {
    let config = Config::load(path)?;
    match command {
        DirectoryCommand::List { json } => {
            if *json {
                println!("{}", serde_json::to_string_pretty(&config.directories)?);
            } else {
                for directory in &config.directories {
                    println!(
                        "{}\t{}\t{:?}\trecursive={}\t{}",
                        directory.id,
                        if directory.enabled {
                            "enabled"
                        } else {
                            "disabled"
                        },
                        directory.mode,
                        directory.recursive,
                        directory.path.display()
                    );
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn history_command(database: &Path, limit: u32, json: bool) -> Result<ExitCode> {
    let entries = HistoryStore::open(database)?.recent(limit)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else {
        for entry in entries {
            println!(
                "{}\t{}\t{} -> {}\t{}{}",
                entry.id,
                entry.state,
                entry.source,
                entry.destination,
                entry.rule_id,
                entry
                    .error
                    .map_or_else(String::new, |error| format!("\t{error}"))
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn starter_config() -> Config {
    Config {
        version: CURRENT_CONFIG_VERSION,
        settings: Settings {
            collision_policy: CollisionPolicy::Skip,
        },
        rules: vec![
            Rule {
                id: "documents".into(),
                enabled: true,
                priority: 10,
                destination: "/files/sorted/documents".into(),
                match_spec: MatchSpec {
                    extensions: vec!["pdf".into(), "docx".into(), "odt".into()],
                    ..MatchSpec::default()
                },
            },
            Rule {
                id: "invoices".into(),
                enabled: true,
                priority: 100,
                destination: "/files/sorted/invoices".into(),
                match_spec: MatchSpec {
                    filename_regexes: vec![r".*invoice.*\.pdf$".into()],
                    ..MatchSpec::default()
                },
            },
        ],
        directories: vec![DirectoryConfig {
            id: "downloads".into(),
            path: "/files/incoming".into(),
            enabled: true,
            mode: ExecutionMode::Once,
            recursive: false,
            schedule: None,
            schedule_timezone: ScheduleTimezone::Local,
            rules: vec!["documents".into(), "invoices".into()],
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn clap_definition_is_consistent() {
        Cli::command().debug_assert();
    }
}
