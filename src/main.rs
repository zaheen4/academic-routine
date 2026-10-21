use academic_routine::cli::{Cli, Commands};
use academic_routine::{
    academic_now, current_academic_day, export_ics_file, load_routine_file, render_day_table,
    render_today_card, render_weekly_overview, DayOfWeek, RoutineState,
};
use chrono::Timelike;
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

    let local_data = PathBuf::from("routine_data.json");
    if local_data.exists() {
        return Some(local_data);
    }

    let local_path = PathBuf::from("final_combined_routine.json");
    if local_path.exists() {
        return Some(local_path);
    }

    let default_scraper_path = PathBuf::from(
        "/home/eyepatch/Documents/projects/routine-to-gsheet/output_of_fetched_routine/final_combined_routine.json",
    );
    if default_scraper_path.exists() {
        return Some(default_scraper_path);
    }

    None
}

fn state_path() -> PathBuf {
    PathBuf::from("routine_state.json")
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    let cli = Cli::parse();
    let state_file = state_path();
    let mut state = RoutineState::load_or_default(&state_file).unwrap_or_default();

    // Handle commands that don't strictly require reading full routine upfront
    match &cli.command {
        Some(Commands::Complete { code }) | Some(Commands::Drop { code }) => {
            if state.complete_course(code) {
                if let Err(e) = state.save(&state_file) {
                    eprintln!("{} Failed to save state: {e}", "❌".red());
                } else {
                    println!(
                        "{} Marked course {} as completed. Excluded from schedules and calendar exports.",
                        "🏁".green().bold(),
                        code.to_uppercase().bold()
                    );
                    println!("   Active completed courses: {:?}", state.dropped_courses);
                }
            } else {
                println!(
                    "{} Course {} is already marked as completed.",
                    "ℹ️".cyan(),
                    code.to_uppercase().bold()
                );
            }
            return;
        }
        Some(Commands::Resume { code }) | Some(Commands::Restore { code }) => {
            if state.resume_course(code) {
                if let Err(e) = state.save(&state_file) {
                    eprintln!("{} Failed to save state: {e}", "❌".red());
                } else {
                    println!(
                        "{} Resumed course {}. It will now appear in your schedule and calendar exports.",
                        "✅".green().bold(),
                        code.to_uppercase().bold()
                    );
                }
            } else {
                println!(
                    "{} Course {} was not in the completed list.",
                    "ℹ️".yellow(),
                    code.to_uppercase().bold()
                );
            }
            return;
        }
        Some(Commands::Completed) | Some(Commands::Dropped) => {
            if state.dropped_courses.is_empty() {
                println!("{} No courses are currently marked as completed.", "ℹ️".green());
            } else {
                let raw_sched = resolve_routine_path(cli.file.as_ref()).and_then(|p| load_routine_file(&p).ok());
                println!("{} Currently completed courses:", "🎓".cyan().bold());
                for c in &state.dropped_courses {
                    let title_opt = raw_sched
                        .as_ref()
                        .and_then(|s| s.entries.iter().find(|e| academic_routine::codes_match(&e.code, c)))
                        .map(|e| e.title.as_str());
                    if let Some(t) = title_opt {
                        println!("   • {}: {}", c.bold(), t.cyan());
                    } else {
                        println!("   • {}", c.bold());
                    }
                }
            }
            return;
        }
        Some(Commands::Status) => {
            println!("============================================================");
            println!("              🎓 Academic Routine Subsystem Status          ");
            println!("============================================================");

            let routine_opt = resolve_routine_path(cli.file.as_ref());
            match routine_opt {
                Some(ref p) => {
                    if let Ok(raw) = load_routine_file(p) {
                        let completed_count = state.dropped_courses.len();
                        let active_count = raw.filter_active(&state.dropped_courses).entries.len();
                        println!(
                            "📁 Routine Data       : {} ({}, {} total classes)",
                            p.display().to_string().cyan(),
                            raw.semester_name.yellow().bold(),
                            raw.entries.len()
                        );
                        println!(
                            "🛡️ Courses Active     : {} ongoing ({} completed)",
                            active_count.to_string().green().bold(),
                            completed_count.to_string().cyan()
                        );
                    } else {
                        println!("📁 Routine Data       : {} (Parse Error)", p.display().to_string().red());
                    }
                }
                None => println!("📁 Routine Data       : {}", "None found".red()),
            }

            if state.is_vacation_active() {
                if let Some(until) = state.vacation_until {
                    println!("🏖️ Vacation Mode      : {} (until {})", "ACTIVE".yellow().bold(), until);
                } else {
                    println!("🏖️ Vacation Mode      : {}", "ACTIVE (Semester Break)".yellow().bold());
                }
            } else {
                println!("🏖️ Vacation Mode      : {}", "INACTIVE (Semester classes ongoing)".green());
            }

            let chat_id_str = std::env::var("TELEGRAM_CHAT_ID").unwrap_or_else(|_| "Not configured".to_string());
            let env_username = std::env::var("TELEGRAM_USERNAME").ok();
            let resolved_user = state.telegram_username.as_ref().or(env_username.as_ref());
            let user_info = resolved_user
                .map(|u| format!("{u} [Chat {chat_id_str}]"))
                .unwrap_or_else(|| format!("Chat {chat_id_str}"));
            if state.morning_alerts {
                println!("🔔 Morning Alerts     : {} (06:00 AM to {})", "ENABLED".green().bold(), user_info.cyan());
            } else {
                println!("🔕 Morning Alerts     : {}", "PAUSED".yellow().bold());
            }

            println!("\n--- Subsystems & Integrations ---");
            println!("🖥️  Offline CLI       : {}", "READY".green().bold());

            let ics_path = PathBuf::from("routine.ics");
            if ics_path.exists() {
                println!("📅 Calendar (.ics)    : {} ({})", "GENERATED".green().bold(), ics_path.display().to_string().cyan());
            } else {
                println!("📅 Calendar (.ics)    : {}", "Not yet generated (run export)".yellow());
            }

            if let Some(ref gid) = state.gist_id {
                let status_str = if state.auto_sync_gist { "ACTIVE".green().bold() } else { "PAUSED".yellow().bold() };
                println!("🔗 GitHub Gist Feed   : {} ({gid})", status_str);
            } else {
                println!("🔗 GitHub Gist Feed   : {}", "Not linked (set-gist <ID>)".yellow());
            }

            let sheet_name = std::env::var("SPREADSHEET_NAME")
                .unwrap_or_else(|_| "CSE-03_B_ClassRoutine".to_string());
            let status_sheet = if state.auto_sync_gsheet { "ACTIVE".green().bold() } else { "PAUSED".yellow().bold() };
            println!("📊 Google Sheets Sync : {} ({sheet_name})", status_sheet);

            let timer_active = std::process::Command::new("systemctl")
                .args(["--user", "is-active", "academic-routine-sync.timer"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "active")
                .unwrap_or(false);
            if timer_active {
                println!("⏱️ Weekly Auto-Timer : {} (Saturday 20:00:00)", "ACTIVE".green().bold());
            } else {
                println!("⏱️ Weekly Auto-Timer : {}", "INACTIVE / PAUSED".yellow().bold());
            }

            println!("🤖 Telegram Bot       : {}", "ONLINE (Local daemon / Render suspended)".green().bold());
            println!("============================================================");
            return;
        }
        Some(Commands::Vacation { action, until }) => {
            let act = action.as_deref().unwrap_or("status").to_lowercase();
            match act.as_str() {
                "on" | "enable" | "start" => {
                    let end_date = if let Some(d_str) = until {
                        match chrono::NaiveDate::parse_from_str(d_str, "%Y-%m-%d") {
                            Ok(d) => Some(d),
                            Err(e) => {
                                eprintln!("{} Invalid date format (expected YYYY-MM-DD): {e}", "❌".red());
                                return;
                            }
                        }
                    } else {
                        None
                    };
                    state.set_vacation(true, end_date);
                    let _ = state.save(&state_file);
                    if let Some(d) = end_date {
                        println!("{} Vacation mode activated until {d}. Morning alerts and class reminders are paused.", "🏖️".yellow().bold());
                    } else {
                        println!("{} Vacation mode activated. Morning alerts and class reminders are paused.", "🏖️".yellow().bold());
                    }
                }
                "off" | "disable" | "stop" | "resume" => {
                    state.set_vacation(false, None);
                    let _ = state.save(&state_file);
                    println!("{} Vacation mode deactivated. Regular schedule notifications are active.", "🎒".green().bold());
                }
                _ => {
                    if state.is_vacation_active() {
                        if let Some(until) = state.vacation_until {
                            println!("🏖️ Vacation mode is ACTIVE until {}", until.to_string().cyan().bold());
                        } else {
                            println!("🏖️ Vacation mode is ACTIVE (Semester Break)");
                        }
                    } else {
                        println!("🎒 Vacation mode is INACTIVE (Normal semester classes ongoing)");
                    }
                    println!("   Use '{}' or '{}' to toggle.", "academic-routine vacation on".cyan(), "academic-routine vacation off".cyan());
                }
            }
            return;
        }
        Some(Commands::Alerts { action }) => {
            let act = action.as_deref().unwrap_or("status").to_lowercase();
            match act.as_str() {
                "off" | "disable" | "pause" | "stop" => {
                    state.set_morning_alerts(false);
                    let _ = state.save(&state_file);
                    println!("{} 06:00 AM proactive morning briefings PAUSED.", "🔕".yellow().bold());
                }
                "on" | "enable" | "resume" | "start" => {
                    state.set_morning_alerts(true);
                    let _ = state.save(&state_file);
                    println!("{} 06:00 AM proactive morning briefings RESUMED.", "🔔".green().bold());
                }
                _ => {
                    let user_info = state.telegram_username.as_deref()
                        .map(|u| format!(" ({u})"))
                        .unwrap_or_default();
                    if state.morning_alerts {
                        println!("🔔 Morning briefings: ENABLED (06:00 AM on class days){user_info}");
                    } else {
                        println!("🔕 Morning briefings: PAUSED{user_info}");
                    }
                    println!("   Use '{}' or '{}' to toggle.", "academic-routine alerts on".cyan(), "academic-routine alerts off".cyan());
                }
            }
            return;
        }
        Some(Commands::Pause { target }) => {
            let tgt = target.as_deref().unwrap_or("all").to_lowercase();
            match tgt.as_str() {
                "alerts" => {
                    state.set_morning_alerts(false);
                    let _ = state.save(&state_file);
                    println!("{} Morning alerts paused.", "🔕".yellow().bold());
                }
                "gist" => {
                    state.set_sync_gist(false);
                    let _ = state.save(&state_file);
                    println!("{} Automatic GitHub Gist sync paused.", "⏸️".yellow().bold());
                }
                "gsheet" => {
                    state.set_sync_gsheet(false);
                    let _ = state.save(&state_file);
                    println!("{} Automatic Google Sheets sync paused.", "⏸️".yellow().bold());
                }
                _ => {
                    state.set_morning_alerts(false);
                    state.set_vacation(true, None);
                    let _ = state.save(&state_file);
                    println!("{} All alerts and automated activities PAUSED (Vacation mode on).", "⏸️".yellow().bold());
                }
            }
            return;
        }
        Some(Commands::Unpause { target }) => {
            let tgt = target.as_deref().unwrap_or("all").to_lowercase();
            match tgt.as_str() {
                "alerts" => {
                    state.set_morning_alerts(true);
                    let _ = state.save(&state_file);
                    println!("{} Morning alerts resumed.", "🔔".green().bold());
                }
                "gist" => {
                    state.set_sync_gist(true);
                    let _ = state.save(&state_file);
                    println!("{} Automatic GitHub Gist sync resumed.", "▶️".green().bold());
                }
                "gsheet" => {
                    state.set_sync_gsheet(true);
                    let _ = state.save(&state_file);
                    println!("{} Automatic Google Sheets sync resumed.", "▶️".green().bold());
                }
                _ => {
                    state.set_morning_alerts(true);
                    state.set_vacation(false, None);
                    let _ = state.save(&state_file);
                    println!("{} System RESUMED (Normal semester operations active).", "▶️".green().bold());
                }
            }
            return;
        }
        Some(Commands::Timer { action }) => {
            let act = action.as_deref().unwrap_or("status").to_lowercase();
            match act.as_str() {
                "pause" | "stop" | "disable" => {
                    let res = std::process::Command::new("systemctl")
                        .args(["--user", "stop", "academic-routine-sync.timer"])
                        .status();
                    match res {
                        Ok(s) if s.success() => println!("{} Weekly auto-sync timer PAUSED.", "⏸️".yellow().bold()),
                        _ => eprintln!("{} Could not pause systemd timer (systemctl --user unavailable or failed).", "⚠️".yellow()),
                    }
                }
                "resume" | "start" | "enable" => {
                    let res = std::process::Command::new("systemctl")
                        .args(["--user", "start", "academic-routine-sync.timer"])
                        .status();
                    match res {
                        Ok(s) if s.success() => println!("{} Weekly auto-sync timer RESUMED (runs Saturdays at 20:00).", "▶️".green().bold()),
                        _ => eprintln!("{} Could not resume systemd timer (systemctl --user unavailable or failed).", "⚠️".yellow()),
                    }
                }
                "run" | "trigger" => {
                    println!("🚀 Triggering immediate execution of automated sync service...");
                    let res = std::process::Command::new("systemctl")
                        .args(["--user", "start", "academic-routine-sync.service"])
                        .status();
                    match res {
                        Ok(s) if s.success() => println!("{} Sync service launched in background.", "✅".green().bold()),
                        _ => eprintln!("{} Could not trigger service.", "❌".red()),
                    }
                }
                _ => {
                    let is_active = std::process::Command::new("systemctl")
                        .args(["--user", "is-active", "academic-routine-sync.timer"])
                        .output()
                        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "active")
                        .unwrap_or(false);
                    if is_active {
                        println!("⏱️ Weekly sync timer: {} (Runs Saturday 20:00:00)", "ACTIVE".green().bold());
                    } else {
                        println!("⏱️ Weekly sync timer: {}", "INACTIVE / PAUSED".yellow().bold());
                    }
                    println!("   Commands: '{}', '{}', '{}'", "timer pause".cyan(), "timer resume".cyan(), "timer run".cyan());
                }
            }
            return;
        }
        Some(Commands::SetGist { id }) => {
            let clean_id = id.trim().to_string();
            state.gist_id = Some(clean_id.clone());
            if let Err(e) = state.save(&state_file) {
                eprintln!("{} Failed to save state: {e}", "❌".red());
            } else {
                println!(
                    "{} Linked GitHub Gist ID: {}",
                    "🔗".green().bold(),
                    clean_id.cyan().bold()
                );
                println!(
                    "   Raw subscription URL:\n   {}",
                    format!(
                        "https://gist.githubusercontent.com/raw/{}/routine.ics",
                        clean_id
                    )
                    .yellow()
                    .bold()
                );
            }
            return;
        }
        Some(Commands::SyncUcam {
            force: _,
            headed,
            local_only,
            no_gist,
            no_gsheet,
        }) => {
            println!(
                "🌐 {}",
                "Connecting to UCAM portal & extracting live routine..."
                    .cyan()
                    .bold()
            );
            let target_json = cli
                .file
                .clone()
                .unwrap_or_else(|| PathBuf::from("routine_data.json"));

            match academic_routine::run_ucam_scraper_opts(&target_json, *headed) {
                Ok(path) => {
                    println!(
                        "{} Successfully extracted routine from UCAM!",
                        "✅".green().bold()
                    );
                    println!("   • Saved to: {}", path.display().to_string().cyan());

                    if let Ok(raw_sched) = load_routine_file(&path) {
                        println!("   • Total classes extracted: {}", raw_sched.entries.len());
                        println!("   • Semester: {}", raw_sched.semester_name.yellow().bold());

                        // CRITICAL FIX: Filter completed courses before exporting routine.ics!
                        let active_sched = raw_sched.filter_active(&state.dropped_courses);
                        let ics_path = PathBuf::from("routine.ics");
                        let _ = export_ics_file(&active_sched, &ics_path, state.semester_end_date);
                        println!("   • Exported active calendar to: {}", ics_path.display().to_string().cyan());

                        if *local_only {
                            println!("🔒 Local-only mode active: skipping cloud Gist and Google Sheets sync.");
                            return;
                        }

                        // Gist sync
                        if !*no_gist && state.auto_sync_gist {
                            if let Some(ref gid) = state.gist_id {
                                println!("🔄 Syncing routine data, calendar, and state to GitHub Gist...");
                                match academic_routine::sync_gist_bundle_with_state(
                                    gid,
                                    &ics_path,
                                    &path,
                                    Some(&state_file),
                                )
                                .await
                                {
                                    Ok(url) => {
                                        println!("🚀 Live subscription feed updated: {}", url.yellow().bold());
                                        println!("   • Synced routine.ics, routine_data.json, and routine_state.json to Gist!");
                                    }
                                    Err(e) => eprintln!("⚠️ Gist sync notice: {e}"),
                                }
                            }
                        } else if *no_gist {
                            println!("ℹ️ Gist sync skipped (--no-gist).");
                        }

                        // Google Sheets sync
                        if !*no_gsheet && state.auto_sync_gsheet {
                            let service_key_candidates = [
                                PathBuf::from("google_cloud_keys/service_account_key.json"),
                                PathBuf::from("/home/eyepatch/Documents/projects/routine-to-gsheet/google_cloud_keys/service_account_key.json"),
                            ];
                            let has_service_key = std::env::var("GOOGLE_SERVICE_ACCOUNT_KEY")
                                .map(PathBuf::from)
                                .map(|p| p.exists())
                                .unwrap_or(false)
                                || service_key_candidates.iter().any(|p| p.exists());

                            if has_service_key {
                                let sheet_name = std::env::var("SPREADSHEET_NAME")
                                    .unwrap_or_else(|_| "CSE-03_B_ClassRoutine".to_string());
                                println!("📊 Syncing routine to Google Spreadsheet ({sheet_name})...");
                                match academic_routine::run_gsheet_sync(&path) {
                                    Ok(()) => {
                                        println!(
                                            "{} Synced to Google Spreadsheet (NewMain & backend tabs)!",
                                            "✅".green().bold()
                                        );
                                    }
                                    Err(e) => eprintln!("⚠️ Google Sheets sync note: {e}"),
                                }
                            }
                        } else if *no_gsheet {
                            println!("ℹ️ Google Sheets sync skipped (--no-gsheet).");
                        }
                    }
                }
                Err(e) => {
                    eprintln!(
                        "{} Failed to extract routine from UCAM: {e}",
                        "❌".red().bold()
                    );
                    std::process::exit(1);
                }
            }
            return;
        }
        Some(Commands::SyncGsheet { file }) => {
            let target_json = file
                .clone()
                .or_else(|| resolve_routine_path(cli.file.as_ref()))
                .unwrap_or_else(|| PathBuf::from("routine_data.json"));

            let sheet_name = std::env::var("SPREADSHEET_NAME")
                .unwrap_or_else(|_| "CSE-03_B_ClassRoutine".to_string());
            println!(
                "📊 Syncing routine from {} to Google Spreadsheet ({sheet_name})...",
                target_json.display().to_string().cyan()
            );

            match academic_routine::run_gsheet_sync(&target_json) {
                Ok(()) => {
                    println!(
                        "{} Successfully updated Google Spreadsheet (NewMain & backend)! 🎉",
                        "✅".green().bold()
                    );
                }
                Err(e) => {
                    eprintln!(
                        "{} Failed to sync Google Spreadsheet: {e}",
                        "❌".red().bold()
                    );
                    std::process::exit(1);
                }
            }
            return;
        }
        Some(Commands::Bot { no_http }) => {
            let routine_path = resolve_routine_path(cli.file.as_ref())
                .unwrap_or_else(|| PathBuf::from("routine_data.json"));

            let ctx = academic_routine::BotContext {
                routine_path,
                state_path: state_file,
                section: cli
                    .section
                    .or_else(|| std::env::var("DEFAULT_SECTION").ok()),
                no_http: *no_http,
            };

            if let Err(e) = academic_routine::run_bot_daemon(ctx).await {
                eprintln!("{} {e}", "❌".red());
                std::process::exit(1);
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
            let today = current_academic_day();
            let now = academic_now().time();
            print!("{}", render_today_card(&schedule, today, now));
        }
        Some(Commands::Next) => {
            let today = current_academic_day();
            let now = academic_now().time();
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
        Some(Commands::Export {
            ics,
            until,
            sync_gist,
            include_all,
        }) => {
            let end_date = if let Some(ref d_str) = until {
                match chrono::NaiveDate::parse_from_str(d_str, "%Y-%m-%d") {
                    Ok(d) => Some(d),
                    Err(e) => {
                        eprintln!(
                            "{} Invalid --until date format (expected YYYY-MM-DD): {e}",
                            "❌".red()
                        );
                        std::process::exit(1);
                    }
                }
            } else {
                state.semester_end_date
            };

            let sched_to_export = if include_all {
                &raw_schedule
            } else {
                &schedule
            };

            match export_ics_file(sched_to_export, &ics, end_date) {
                Ok(()) => {
                    println!(
                        "{} Generated iCalendar feed at {}",
                        "📅".green().bold(),
                        ics.display().to_string().cyan().bold()
                    );
                    println!("   • Active classes exported: {}", sched_to_export.entries.len());
                    if let Some(until) = end_date {
                        println!("   • Automatic expiry (UNTIL): {}", until);
                    }
                    println!("   • Alarms included: 30-minute sound & notification popups");

                    // Check if Gist sync requested or configured
                    if sync_gist || state.gist_id.is_some() {
                        if let Some(ref gid) = state.gist_id {
                            println!("{} Syncing to GitHub Gist...", "🔄".cyan());
                            match academic_routine::sync_gist_file(gid, &ics).await {
                                Ok(url) => {
                                    println!(
                                        "{} Live subscription feed updated!",
                                        "🚀".green().bold()
                                    );
                                    println!(
                                        "   Subscribe in Google Calendar: {}",
                                        url.yellow().bold()
                                    );
                                }
                                Err(e) => eprintln!("{} {e}", "❌".red()),
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("{} Failed to write .ics file: {e}", "❌".red());
                    std::process::exit(1);
                }
            }
        }
        Some(Commands::SyncGist) => {
            let gid = match state.gist_id.as_deref() {
                Some(id) => id,
                None => {
                    eprintln!(
                        "{} No Gist linked. Run {} first.",
                        "❌".red(),
                        "academic-routine set-gist <ID>".cyan()
                    );
                    std::process::exit(1);
                }
            };

            let ics_path = PathBuf::from("routine.ics");
            if !ics_path.exists() {
                let _ = export_ics_file(&schedule, &ics_path, state.semester_end_date);
            }

            println!("{} Syncing routine, calendar, and state to GitHub Gist...", "🔄".cyan());
            let json_path = PathBuf::from("routine_data.json");
            let result = if json_path.exists() {
                academic_routine::sync_gist_bundle_with_state(gid, &ics_path, &json_path, Some(&state_file)).await
            } else {
                academic_routine::sync_gist_file(gid, &ics_path).await
            };

            match result {
                Ok(url) => {
                    println!("{} Live subscription feed updated!", "🚀".green().bold());
                    println!("   Google Calendar Feed: {}", url.yellow().bold());
                }
                Err(e) => eprintln!("{} {e}", "❌".red()),
            }
        }
        None => {
            // Default action if no subcommand is given: print weekly overview
            print!("{}", render_weekly_overview(&schedule));
        }
        _ => {}
    }
}
