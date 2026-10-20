use academic_routine::cli::{Cli, Commands};
use academic_routine::{
    load_routine_file, render_day_table, render_today_card, render_weekly_overview, DayOfWeek,
    RoutineState,
};
use chrono::{Datelike, Local, Timelike};
use clap::Parser;
use colored::Colorize;
use std::path::PathBuf;
use std::str::FromStr;

fn resolve_routine_path(explicit: Option<&PathBuf>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        if p.exists() {
            return Some(p.clone());
        }
    }

    let default_scraper_path = PathBuf::from(
        "/home/eyepatch/Documents/projects/routine-to-gsheet/output_of_fetched_routine/final_combined_routine.json",
    );
    if default_scraper_path.exists() {
        return Some(default_scraper_path);
    }

    let local_path = PathBuf::from("final_combined_routine.json");
    if local_path.exists() {
        return Some(local_path);
    }

    None
}

fn state_path() -> PathBuf {
    PathBuf::from("routine_state.json")
}

fn current_day_of_week() -> Option<DayOfWeek> {
    let now = Local::now();
    match now.weekday() {
        chrono::Weekday::Sat => Some(DayOfWeek::Sat),
        chrono::Weekday::Sun => Some(DayOfWeek::Sun),
        chrono::Weekday::Mon => Some(DayOfWeek::Mon),
        chrono::Weekday::Tue => Some(DayOfWeek::Tue),
        chrono::Weekday::Wed => Some(DayOfWeek::Wed),
        chrono::Weekday::Thu => Some(DayOfWeek::Thu),
        chrono::Weekday::Fri => Some(DayOfWeek::Fri),
    }
}

fn main() {
    let cli = Cli::parse();
    let state_file = state_path();
    let mut state = RoutineState::load_or_default(&state_file).unwrap_or_default();

    // Handle course lifecycle commands that don't require loading routine data
    match &cli.command {
        Some(Commands::Drop { code }) => {
            if state.drop_course(code) {
                if let Err(e) = state.save(&state_file) {
                    eprintln!("{} Failed to save state: {e}", "❌".red());
                } else {
                    println!(
                        "{} Dropped course {}. It is now excluded from schedules and calendar exports.",
                        "✅".green(),
                        code.to_uppercase().bold()
                    );
                    println!("   Active dropped courses: {:?}", state.dropped_courses);
                }
            } else {
                println!(
                    "{} Course {} is already marked as dropped.",
                    "ℹ️".cyan(),
                    code.to_uppercase().bold()
                );
            }
            return;
        }
        Some(Commands::Restore { code }) => {
            if state.restore_course(code) {
                if let Err(e) = state.save(&state_file) {
                    eprintln!("{} Failed to save state: {e}", "❌".red());
                } else {
                    println!(
                        "{} Restored course {}. It will now appear in your schedule and calendar exports.",
                        "✅".green(),
                        code.to_uppercase().bold()
                    );
                }
            } else {
                println!(
                    "{} Course {} was not in the dropped list.",
                    "ℹ️".yellow(),
                    code.to_uppercase().bold()
                );
            }
            return;
        }
        Some(Commands::Dropped) => {
            if state.dropped_courses.is_empty() {
                println!("{} No courses are currently dropped.", "ℹ️".green());
            } else {
                println!("{} Currently dropped courses:", "🚫".yellow());
                for c in &state.dropped_courses {
                    println!("   • {}", c.bold());
                }
            }
            return;
        }
        _ => {}
    }

    // Resolve routine file path
    let routine_path = match resolve_routine_path(cli.file.as_ref()) {
        Some(p) => p,
        None => {
            eprintln!(
                "{} No routine JSON file found. Specify one with {} or place it at {}",
                "❌".red().bold(),
                "--file <PATH>".cyan(),
                "final_combined_routine.json".yellow()
            );
            std::process::exit(1);
        }
    };

    // Ingest routine
    let raw_schedule = match load_routine_file(&routine_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "{} Failed to parse routine from {}: {e}",
                "❌".red(),
                routine_path.display()
            );
            std::process::exit(1);
        }
    };

    // Apply dropped course filter
    let mut schedule = raw_schedule.filter_active(&state.dropped_courses);

    // Apply section filter if requested
    if let Some(ref sec) = cli.section {
        schedule = schedule.filter_section(sec);
    }

    // Dispatch subcommands
    match cli.command {
        Some(Commands::View { day }) => {
            if let Some(day_str) = day {
                match DayOfWeek::from_str(&day_str) {
                    Ok(d) => print!("{}", render_day_table(&schedule, d)),
                    Err(e) => eprintln!("{} {e}", "❌".red()),
                }
            } else {
                print!("{}", render_weekly_overview(&schedule));
            }
        }
        Some(Commands::Today) => {
            let today = current_day_of_week().unwrap_or(DayOfWeek::Sat);
            let now = Local::now().time();
            print!("{}", render_today_card(&schedule, today, now));
        }
        Some(Commands::Next) => {
            let today = current_day_of_week().unwrap_or(DayOfWeek::Sat);
            let now = Local::now().time();
            let classes = schedule.for_day(today);

            if let Some(active) = classes.iter().find(|c| c.time_slot.contains_time(now)) {
                println!(
                    "{} Currently in session: [{}] {} in Room {} ({} - {})",
                    "🔴".red(),
                    active.section.bold(),
                    active.title.bold(),
                    active.room.green(),
                    active.time_slot.start().format("%I:%M %p"),
                    active.time_slot.end().format("%I:%M %p")
                );
            } else if let Some(next) = classes.iter().find(|c| c.time_slot.start() > now) {
                let mins = (next.time_slot.start().hour() * 60 + next.time_slot.start().minute())
                    .saturating_sub(now.hour() * 60 + now.minute());
                println!(
                    "{} Next class: [{}] {} in Room {} at {} (starts in {} mins)",
                    "🟢".green(),
                    next.section.bold(),
                    next.title.bold(),
                    next.room.green(),
                    next.time_slot.start().format("%I:%M %p"),
                    mins.to_string().cyan().bold()
                );
                if !next.teacher.name.is_empty() {
                    println!("   └─ {}", next.teacher.formatted_summary());
                }
            } else {
                println!("{} No more classes scheduled for today!", "🏁".cyan());
            }
        }
        Some(Commands::Conflicts) => {
            let conflicts = schedule.find_conflicts();
            if conflicts.is_empty() {
                println!(
                    "{} Zero scheduling conflicts detected in active routine!",
                    "✅".green().bold()
                );
            } else {
                println!(
                    "{} Found {} conflict(s) in active schedule:",
                    "⚠️".yellow().bold(),
                    conflicts.len()
                );
                for c in &conflicts {
                    println!("   • [{}] {}", c.day.to_string().cyan(), c.reason);
                }
            }
        }
        Some(Commands::Teachers) => {
            let dir = schedule.teacher_directory();
            println!("📚 Instructor Directory ({} teachers):", dir.len());
            for (name, contact) in dir {
                println!("   • {name}");
                if let Some(phone) = contact.phone {
                    println!("     ☎ Phone: {phone}");
                }
                if let Some(email) = contact.email {
                    println!("     ✉ Email: {email}");
                }
            }
        }
        None => {
            // Default action if no subcommand is given: print weekly overview
            print!("{}", render_weekly_overview(&schedule));
        }
        _ => {}
    }
}
