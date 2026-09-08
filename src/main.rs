use std::{collections::BTreeMap, fs, path::PathBuf, process::ExitCode};

use anyhow::{Context, Result, ensure};
use chrono::NaiveDate;
use clap::{Parser, Subcommand};
use hydrant_optimizer::{adapter, app, model::*, storage, tui};

#[derive(Parser)]
#[command(
    version,
    about = "Exact local MIT course scheduler. Run without a subcommand to open the terminal UI."
)]
struct Cli {
    /// Cache and separate manual overlay directory.
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    /// Use only a previously fetched catalog.
    #[arg(long, global = true)]
    offline: bool,
    /// Read a Hydrant JSON snapshot rather than the network (requires --term).
    #[arg(long, global = true, requires = "term")]
    catalog: Option<PathBuf>,
    /// Matching latestTerm.json metadata for --catalog.
    #[arg(long, global = true, requires = "catalog")]
    term: Option<PathBuf>,
    /// Calendar output path. Existing files are never overwritten.
    #[arg(long, global = true, default_value = "schedule.ics")]
    output: PathBuf,
    /// Machine-readable output for non-TUI commands.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Open the terminal UI, optionally preselecting subjects.
    Tui {
        #[arg(long)]
        select: Vec<String>,
    },
    /// Search subject numbers and titles.
    Search {
        #[arg(default_value = "")]
        query: String,
    },
    /// Download and validate the latest catalog, preserving manual edits.
    Refresh,
    /// Find an exact optimum for these fixed subjects.
    Optimize {
        #[arg(required = true)]
        subjects: Vec<String>,
        /// Also write the local calendar file.
        #[arg(long)]
        export: bool,
        /// Select an interchangeable actual section: requirement_id=option_id.
        #[arg(long = "member")]
        members: Vec<String>,
    },
    /// Manage term-scoped manual sections without modifying downloaded data.
    Manual {
        #[command(subcommand)]
        command: ManualCommand,
    },
}

#[derive(Subcommand)]
enum ManualCommand {
    List,
    Add {
        #[arg(long)]
        course: String,
        #[arg(long)]
        kind: String,
        #[arg(long, default_value = "Manual section")]
        label: String,
        #[arg(long, default_value = "")]
        room: String,
        /// Atomic bundle, e.g. 'Mon 09:00-10:00;Wed 09:00-10:00'.
        #[arg(long)]
        meetings: String,
        /// Replace an existing manual entry, preserving its identity.
        #[arg(long)]
        id: Option<String>,
        /// Preserved, but date-limited options are omitted from weekly optimization.
        #[arg(long)]
        start_date: Option<NaiveDate>,
        #[arg(long)]
        end_date: Option<NaiveDate>,
    },
    Enable {
        id: String,
    },
    Disable {
        id: String,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn default_data_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(path).join("hydrant-optimizer");
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/hydrant-optimizer")
    } else {
        home.join(".local/share/hydrant-optimizer")
    }
}

fn run(cli: Cli) -> Result<u8> {
    let dir = cli.data_dir.unwrap_or_else(default_data_dir);
    // Listing retained edits does not require a network connection or even a cache.
    if matches!(
        cli.command,
        Some(Command::Manual {
            command: ManualCommand::List
        })
    ) {
        let store = storage::load_manual(&dir)?;
        if cli.json {
            println!("{}", serde_json::to_string_pretty(&store)?);
        } else {
            for e in store.entries {
                println!(
                    "{} [{}] {} {} {} {}",
                    e.option.id,
                    if e.enabled { "on" } else { "off" },
                    e.term_id,
                    e.course_id,
                    e.kind,
                    e.option.label
                );
            }
        }
        return Ok(0);
    }
    if matches!(cli.command, Some(Command::Refresh)) {
        ensure!(
            !cli.offline && cli.catalog.is_none(),
            "refresh requires the network, without --offline or --catalog"
        );
    }
    let base = if let Some(catalog) = cli.catalog {
        let term = cli.term.context("--catalog requires --term")?;
        adapter::parse_catalog(
            &fs::read_to_string(&catalog).with_context(|| format!("read {}", catalog.display()))?,
            &fs::read_to_string(&term).with_context(|| format!("read {}", term.display()))?,
        )?
    } else {
        storage::load_dataset(&dir, cli.offline)?
    };
    let mut manual = storage::load_manual(&dir)?;
    match cli.command {
        None | Some(Command::Tui { .. }) => {
            let initial = match cli.command {
                Some(Command::Tui { select }) => select,
                _ => Vec::new(),
            };
            tui::run(base, manual, dir, cli.output, initial)?;
        }
        Some(Command::Refresh) => {
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({"term":base.term_id,"courses":base.courses.len(),"last_updated":base.last_updated,"notices":base.notices})
                );
            } else {
                println!(
                    "{}: {} subjects, source updated {}",
                    base.term_id,
                    base.courses.len(),
                    base.last_updated
                );
                for notice in &base.notices {
                    eprintln!("Notice: {notice}");
                }
            }
        }
        Some(Command::Search { query }) => {
            let query = query.to_lowercase();
            let found: Vec<_> = base
                .courses
                .values()
                .filter(|c| {
                    c.id.to_lowercase().contains(&query) || c.title.to_lowercase().contains(&query)
                })
                .collect();
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&found)?);
            } else {
                for course in found {
                    println!("{}  {}", course.id, course.title);
                }
            }
            for notice in &base.notices {
                eprintln!("Notice: {notice}");
            }
        }
        Some(Command::Optimize {
            subjects,
            export,
            members,
        }) => {
            let dataset = storage::apply_manual(&base, &manual)?;
            let solution = app::optimize(&dataset, &subjects, None)?;
            let mut choices = BTreeMap::new();
            for member in members {
                let (requirement, id) = member
                    .split_once('=')
                    .context("--member must be requirement_id=option_id")?;
                ensure!(
                    !requirement.is_empty() && !id.is_empty(),
                    "--member must be requirement_id=option_id"
                );
                ensure!(
                    choices
                        .insert(requirement.to_string(), id.to_string())
                        .is_none(),
                    "duplicate --member for {requirement}"
                );
            }
            let actual = if solution.status == SolveStatus::OptimalKnown {
                app::actual_sections(&dataset, &solution, &choices)?
            } else {
                Vec::new()
            };
            // Infeasibility is a solver outcome, even when export was requested.
            // Preserve its text/JSON and exit status without touching the output path.
            let report = if export && solution.status == SolveStatus::OptimalKnown {
                Some(app::write_calendar(
                    &dataset,
                    &solution,
                    &choices,
                    &cli.output,
                )?)
            } else {
                None
            };
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &serde_json::json!({"solution":solution,"sections":actual,"export":report.as_ref().map(|r|serde_json::json!({"path":cli.output,"events":r.event_count,"notices":r.notices}))})
                    )?
                );
            } else {
                match solution.status {
                    SolveStatus::OptimalKnown => {
                        let score = solution.score.context("missing optimum score")?;
                        println!(
                            "Optimal over known supported meetings: {} occupied days, {} gap minutes",
                            score.occupied_days, score.gap_minutes
                        );
                        let mut rows = Vec::new();
                        for section in &actual {
                            for m in &section.section.meetings {
                                rows.push((
                                    m.weekday,
                                    m.start_minute,
                                    format!(
                                        "{}  {} {}  {}  {}",
                                        m.display(),
                                        section.course_id,
                                        section.kind,
                                        section.section.label,
                                        section.section.room
                                    ),
                                ));
                            }
                        }
                        rows.sort();
                        for (_, _, line) in rows {
                            println!("{line}");
                        }
                        for choice in &solution.choices {
                            println!(
                                "{}: {} same-time option(s)",
                                choice.requirement_id,
                                choice.members.len()
                            );
                        }
                    }
                    SolveStatus::Infeasible => println!(
                        "Infeasible: known supported meetings cannot all be selected without conflict."
                    ),
                    SolveStatus::Cancelled => println!("Cancelled: no optimum is claimed."),
                }
                for notice in &solution.unresolved {
                    eprintln!("Notice: {notice}");
                }
                if let Some(report) = report {
                    println!(
                        "Exported {} events to {}",
                        report.event_count,
                        cli.output.display()
                    );
                    for notice in report.notices {
                        eprintln!("Export notice: {notice}");
                    }
                }
            }
            return Ok(if solution.status == SolveStatus::Infeasible {
                2
            } else {
                0
            });
        }
        Some(Command::Manual { command }) => {
            for notice in &base.notices {
                eprintln!("Notice: {notice}");
            }
            let id = match command {
                ManualCommand::Add {
                    course,
                    kind,
                    label,
                    room,
                    meetings,
                    id,
                    start_date,
                    end_date,
                } => {
                    if let Some(ref id) = id {
                        ensure!(
                            manual
                                .entries
                                .iter()
                                .any(|e| e.term_id == base.term_id && &e.option.id == id),
                            "manual section not found in this term: {id}"
                        );
                    }
                    let mut meetings = storage::parse_meetings(&meetings)?;
                    for meeting in &mut meetings {
                        meeting.start_date = start_date;
                        meeting.end_date = end_date;
                    }
                    let mut entry = app::manual_entry(
                        &base,
                        &course,
                        &kind,
                        &label,
                        &room,
                        meetings,
                        id.as_deref(),
                    )?;
                    let id = entry.option.id.clone();
                    if let Some(existing) = manual
                        .entries
                        .iter_mut()
                        .find(|e| e.term_id == entry.term_id && e.option.id == id)
                    {
                        ensure!(
                            existing.course_id == entry.course_id && existing.kind == entry.kind,
                            "editing cannot change a manual section's subject or component; add a new entry instead"
                        );
                        entry.enabled = existing.enabled;
                        *existing = entry;
                    } else {
                        manual.entries.push(entry);
                    }
                    id
                }
                ManualCommand::Enable { id } => {
                    let entry = manual
                        .entries
                        .iter_mut()
                        .find(|e| e.term_id == base.term_id && e.option.id == id)
                        .context("manual section not found in this term")?;
                    entry.enabled = true;
                    id
                }
                ManualCommand::Disable { id } => {
                    let entry = manual
                        .entries
                        .iter_mut()
                        .find(|e| e.term_id == base.term_id && e.option.id == id)
                        .context("manual section not found in this term")?;
                    entry.enabled = false;
                    id
                }
                ManualCommand::List => unreachable!(),
            };
            // Validate the resulting merged state before persisting any edits.
            storage::apply_manual(&base, &manual)?;
            storage::save_manual(&dir, &manual)?;
            if cli.json {
                println!("{}", serde_json::json!({"id":id,"saved":true}));
            } else {
                println!("Saved {id}");
            }
        }
    }
    Ok(0)
}
