use academic_routine::cli::{Cli, Commands};
use academic_routine::{
    academic_now, current_academic_day, export_ics_file, load_routine_file, render_day_table,
    render_teacher_card, render_today_card, render_weekly_overview, resolve_admin_chat_id,
    resolve_routine_path, resolve_write_target, DayOfWeek, StateStore,
};
use chrono::Timelike;
use clap::Parser;
use colored::Colorize;
use std::path::PathBuf;
use std::str::FromStr;

fn state_path() -> PathBuf {
    let output_state = PathBuf::from("output/routine_state.json");
    if output_state.exists() {
        return output_state;
    }

    let local_state = PathBuf::from("routine_state.json");
    if local_state.exists() {
        return local_state;
    }

    output_state
}

fn resolve_ics_path() -> PathBuf {
    let output_ics = PathBuf::from("output/routine.ics");
    if output_ics.exists() {
        return output_ics;
    }

    let local_ics = PathBuf::from("routine.ics");
    if local_ics.exists() {
        return local_ics;
    }

    output_ics
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    let cli = Cli::parse();

    let state_file = state_path();
    let (store, recovery_note) = match StateStore::open(state_file.clone()) {
        Ok(res) => res,
        Err(e) => {
            eprintln!("{} Failed to open state file: {e}", "❌".red());
            std::process::exit(1);
        }
    };
    if let Some(note) = recovery_note {
        eprintln!("⚠️ StateStore notice: {note}");
    }

    // Handle commands that don't strictly require reading full routine upfront
    match &cli.command {
        Some(Commands::Diff { prev, current }) => {
            let prev_path = prev.clone().unwrap_or_else(|| {
                let p = PathBuf::from("output/routine_data.prev.json");
                if p.exists() {
                    p
                } else {
                    PathBuf::from("routine_data.prev.json")
                }
            });
            let curr_path = match current.clone() {
                Some(p) => p,
                None => match resolve_routine_path(cli.file.as_ref()) {
                    Ok(Some(p)) => p,
                    Ok(None) => PathBuf::from("output/routine_data.json"),
                    Err(e) => {
                        eprintln!("{} {e}", "❌".red().bold());
                        std::process::exit(1);
                    }
                },
            };

            if !prev_path.exists() {
                eprintln!(
                    "{} Previous routine baseline not found at '{}'. Run a sync first to establish a baseline.",
                    "❌".red(),
                    prev_path.display()
                );
                return;
            }
            if !curr_path.exists() {
                eprintln!(
                    "{} Current routine file not found at '{}'.",
                    "❌".red(),
                    curr_path.display()
                );
                return;
            }

            let old_sched = match load_routine_file(&prev_path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!(
                        "{} Failed to parse '{}': {e}",
                        "❌".red(),
                        prev_path.display()
                    );
                    return;
                }
            };
            let new_sched = match load_routine_file(&curr_path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!(
                        "{} Failed to parse '{}': {e}",
                        "❌".red(),
                        curr_path.display()
                    );
                    return;
                }
            };

            let mut diff = academic_routine::ScheduleDiff::between(&old_sched, &new_sched);
            if let Some(ref sec) = cli.section {
                diff = diff.filter_section(sec);
            }

            print!("{}", diff.render_terminal());
            return;
        }
        Some(Commands::Complete { code }) | Some(Commands::Drop { code }) => {
            match store.update(|s| (s.complete_course(code), s.dropped_courses.clone())) {
                Ok((true, dropped)) => {
                    println!(
                        "{} Marked course {} as completed. Excluded from schedules and calendar exports.",
                        "🏁".green().bold(),
                        code.to_uppercase().bold()
                    );
                    println!("   Active completed courses: {:?}", dropped);
                }
                Ok((false, _)) => {
                    println!(
                        "{} Course {} is already marked as completed.",
                        "ℹ️".cyan(),
                        code.to_uppercase().bold()
                    );
                }
                Err(e) => {
                    eprintln!("{} Failed to save state: {e}", "❌".red());
                }
            }
            return;
        }
        Some(Commands::Resume { code }) | Some(Commands::Restore { code }) => {
            match store.update(|s| s.resume_course(code)) {
                Ok(true) => {
                    println!(
                        "{} Resumed course {}. It will now appear in your schedule and calendar exports.",
                        "✅".green().bold(),
                        code.to_uppercase().bold()
                    );
                }
                Ok(false) => {
                    println!(
                        "{} Course {} was not in the completed list.",
                        "ℹ️".yellow(),
                        code.to_uppercase().bold()
                    );
                }
                Err(e) => {
                    eprintln!("{} Failed to save state: {e}", "❌".red());
                }
            }
            return;
        }
        Some(Commands::Completed) | Some(Commands::Dropped) => {
            let state = store.read();
            if state.dropped_courses.is_empty() {
                println!(
                    "{} No courses are currently marked as completed.",
                    "ℹ️".green()
                );
            } else {
                let raw_sched = match resolve_routine_path(cli.file.as_ref()) {
                    Ok(Some(p)) => load_routine_file(&p).ok(),
                    Ok(None) => None,
                    Err(e) => {
                        eprintln!("{} {e}", "❌".red().bold());
                        std::process::exit(1);
                    }
                };
                println!("{} Currently completed courses:", "🎓".cyan().bold());
                for c in &state.dropped_courses {
                    let title_opt = raw_sched
                        .as_ref()
                        .and_then(|s| {
                            s.entries
                                .iter()
                                .find(|e| academic_routine::codes_match(&e.code, c))
                        })
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
            let state = store.read();
            println!("┌──────────────────────────────────────────────────────────────┐");
            println!("│             🎓 Academic Routine Subsystem Status             │");
            println!("├──────────────────────────────────────────────────────────────┤");

            let routine_opt = match resolve_routine_path(cli.file.as_ref()) {
                Ok(res) => res,
                Err(e) => {
                    eprintln!("{} {e}", "❌".red().bold());
                    std::process::exit(1);
                }
            };
            match routine_opt {
                Some(ref p) => {
                    if let Ok(raw) = load_routine_file(p) {
                        let completed_count = state.dropped_courses.len();
                        let active_count = raw.filter_active(&state.dropped_courses).entries.len();
                        println!(
                            "│ 📁 Routine Data       : {} ({}, {} total classes)",
                            p.display().to_string().cyan(),
                            raw.semester_name.yellow().bold(),
                            raw.entries.len()
                        );
                        println!(
                            "│ 🛡️ Courses Active     : {} ongoing ({} completed)",
                            active_count.to_string().green().bold(),
                            completed_count.to_string().cyan()
                        );
                    } else {
                        println!(
                            "│ 📁 Routine Data       : {} (Parse Error)",
                            p.display().to_string().red()
                        );
                    }
                }
                None => println!("│ 📁 Routine Data       : {}", "None found".red()),
            }

            if state.is_vacation_active() {
                if let Some(until) = state.vacation_until {
                    println!(
                        "│ 🏖️ Vacation Mode      : {} (until {})",
                        "ACTIVE".yellow().bold(),
                        until
                    );
                } else {
                    println!(
                        "│ 🏖️ Vacation Mode      : {}",
                        "ACTIVE (Semester Break)".yellow().bold()
                    );
                }
            } else {
                println!(
                    "│ 🏖️ Vacation Mode      : {}",
                    "INACTIVE (Semester classes ongoing)".green()
                );
            }

            let chat_id_str = match resolve_admin_chat_id() {
                Some(id) => id.to_string(),
                None => "Not configured".to_string(),
            };
            let env_username = std::env::var("TELEGRAM_USERNAME").ok();
            let resolved_user = state.owner_username.as_ref().or(env_username.as_ref());
            let user_info = resolved_user
                .map(|u| format!("{u} [Chat {chat_id_str}]"))
                .unwrap_or_else(|| format!("Chat {chat_id_str}"));
            if state.morning_alerts {
                println!(
                    "│ 🔔 Morning Alerts     : {} (06:00 AM to {})",
                    "ENABLED".green().bold(),
                    user_info.cyan()
                );
            } else {
                println!("│ 🔕 Morning Alerts     : {}", "PAUSED".yellow().bold());
            }

            println!("├──────────────────────────────────────────────────────────────┤");
            println!("│ 🖥️  Offline CLI       : {}", "READY".green().bold());

            let ics_path = resolve_ics_path();
            if ics_path.exists() {
                println!(
                    "│ 📅 Calendar (.ics)    : {} ({})",
                    "GENERATED".green().bold(),
                    ics_path.display().to_string().cyan()
                );
            } else {
                println!(
                    "│ 📅 Calendar (.ics)    : {}",
                    "Not yet generated (run export)".yellow()
                );
            }

            if let Some(ref gid) = state.gist_id {
                let status_str = if state.auto_sync_gist {
                    "ACTIVE".green().bold()
                } else {
                    "PAUSED".yellow().bold()
                };
                println!("│ 🔗 GitHub Gist Feed   : {} ({gid})", status_str);
            } else {
                println!(
                    "│ 🔗 GitHub Gist Feed   : {}",
                    "Not linked (set-gist <ID>)".yellow()
                );
            }

            let sheet_name = std::env::var("SPREADSHEET_NAME")
                .unwrap_or_else(|_| "CSE-03_B_ClassRoutine".to_string());
            let status_sheet = if state.auto_sync_gsheet {
                "ACTIVE".green().bold()
            } else {
                "PAUSED".yellow().bold()
            };
            println!("│ 📊 Google Sheets Sync : {} ({sheet_name})", status_sheet);

            let timer_active = std::process::Command::new("systemctl")
                .args(["--user", "is-active", "academic-routine-sync.timer"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "active")
                .unwrap_or(false);
            if timer_active {
                println!(
                    "│ ⏱️ Weekly Auto-Timer : {} (Saturday 20:00:00)",
                    "ACTIVE".green().bold()
                );
            } else {
                println!(
                    "│ ⏱️ Weekly Auto-Timer : {}",
                    "INACTIVE / PAUSED".yellow().bold()
                );
            }

            println!(
                "│ 🤖 Telegram Bot       : {}",
                "ONLINE (Local daemon)".green().bold()
            );
            println!("└──────────────────────────────────────────────────────────────┘");
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
                                eprintln!(
                                    "{} Invalid date format (expected YYYY-MM-DD): {e}",
                                    "❌".red()
                                );
                                return;
                            }
                        }
                    } else {
                        None
                    };
                    if let Err(e) = store.update(|s| s.set_vacation(true, end_date)) {
                        eprintln!("{} Failed to save state: {e}", "❌".red());
                        return;
                    }
                    if let Some(d) = end_date {
                        println!("{} Vacation mode activated until {d}. Morning alerts and class reminders are paused.", "🏖️".yellow().bold());
                    } else {
                        println!("{} Vacation mode activated. Morning alerts and class reminders are paused.", "🏖️".yellow().bold());
                    }
                }
                "off" | "disable" | "stop" | "resume" => {
                    if let Err(e) = store.update(|s| s.set_vacation(false, None)) {
                        eprintln!("{} Failed to save state: {e}", "❌".red());
                        return;
                    }
                    println!(
                        "{} Vacation mode deactivated. Regular schedule notifications are active.",
                        "🎒".green().bold()
                    );
                }
                _ => {
                    let state = store.read();
                    if state.is_vacation_active() {
                        if let Some(until) = state.vacation_until {
                            println!(
                                "🏖️ Vacation mode is ACTIVE until {}",
                                until.to_string().cyan().bold()
                            );
                        } else {
                            println!("🏖️ Vacation mode is ACTIVE (Semester Break)");
                        }
                    } else {
                        println!("🎒 Vacation mode is INACTIVE (Normal semester classes ongoing)");
                    }
                    println!(
                        "   Use '{}' or '{}' to toggle.",
                        "academic-routine vacation on".cyan(),
                        "academic-routine vacation off".cyan()
                    );
                }
            }
            return;
        }
        Some(Commands::Alerts { action }) => {
            let act = action.as_deref().unwrap_or("status").to_lowercase();
            match act.as_str() {
                "off" | "disable" | "pause" | "stop" => {
                    if let Err(e) = store.update(|s| s.set_morning_alerts(false)) {
                        eprintln!("{} Failed to save state: {e}", "❌".red());
                        return;
                    }
                    println!(
                        "{} 06:00 AM proactive morning briefings PAUSED.",
                        "🔕".yellow().bold()
                    );
                }
                "on" | "enable" | "resume" | "start" => {
                    if let Err(e) = store.update(|s| s.set_morning_alerts(true)) {
                        eprintln!("{} Failed to save state: {e}", "❌".red());
                        return;
                    }
                    println!(
                        "{} 06:00 AM proactive morning briefings RESUMED.",
                        "🔔".green().bold()
                    );
                }
                _ => {
                    let state = store.read();
                    let user_info = state
                        .owner_username
                        .as_deref()
                        .map(|u| format!(" ({u})"))
                        .unwrap_or_default();
                    if state.morning_alerts {
                        println!(
                            "🔔 Morning briefings: ENABLED (06:00 AM on class days){user_info}"
                        );
                    } else {
                        println!("🔕 Morning briefings: PAUSED{user_info}");
                    }
                    println!(
                        "   Use '{}' or '{}' to toggle.",
                        "academic-routine alerts on".cyan(),
                        "academic-routine alerts off".cyan()
                    );
                }
            }
            return;
        }
        Some(Commands::Pause { target }) => {
            let tgt = target.as_deref().unwrap_or("all").to_lowercase();
            match tgt.as_str() {
                "alerts" => {
                    if let Err(e) = store.update(|s| s.set_morning_alerts(false)) {
                        eprintln!("{} Failed to save state: {e}", "❌".red());
                        return;
                    }
                    println!("{} Morning alerts paused.", "🔕".yellow().bold());
                }
                "gist" => {
                    if let Err(e) = store.update(|s| s.set_sync_gist(false)) {
                        eprintln!("{} Failed to save state: {e}", "❌".red());
                        return;
                    }
                    println!(
                        "{} Automatic GitHub Gist sync paused.",
                        "⏸️".yellow().bold()
                    );
                }
                "gsheet" => {
                    if let Err(e) = store.update(|s| s.set_sync_gsheet(false)) {
                        eprintln!("{} Failed to save state: {e}", "❌".red());
                        return;
                    }
                    println!(
                        "{} Automatic Google Sheets sync paused.",
                        "⏸️".yellow().bold()
                    );
                }
                _ => {
                    if let Err(e) = store.update(|s| {
                        s.set_morning_alerts(false);
                        s.set_vacation(true, None);
                    }) {
                        eprintln!("{} Failed to save state: {e}", "❌".red());
                        return;
                    }
                    println!(
                        "{} All alerts and automated activities PAUSED (Vacation mode on).",
                        "⏸️".yellow().bold()
                    );
                }
            }
            return;
        }
        Some(Commands::Unpause { target }) => {
            let tgt = target.as_deref().unwrap_or("all").to_lowercase();
            match tgt.as_str() {
                "alerts" => {
                    if let Err(e) = store.update(|s| s.set_morning_alerts(true)) {
                        eprintln!("{} Failed to save state: {e}", "❌".red());
                        return;
                    }
                    println!("{} Morning alerts resumed.", "🔔".green().bold());
                }
                "gist" => {
                    if let Err(e) = store.update(|s| s.set_sync_gist(true)) {
                        eprintln!("{} Failed to save state: {e}", "❌".red());
                        return;
                    }
                    println!(
                        "{} Automatic GitHub Gist sync resumed.",
                        "▶️".green().bold()
                    );
                }
                "gsheet" => {
                    if let Err(e) = store.update(|s| s.set_sync_gsheet(true)) {
                        eprintln!("{} Failed to save state: {e}", "❌".red());
                        return;
                    }
                    println!(
                        "{} Automatic Google Sheets sync resumed.",
                        "▶️".green().bold()
                    );
                }
                _ => {
                    if let Err(e) = store.update(|s| {
                        s.set_morning_alerts(true);
                        s.set_vacation(false, None);
                    }) {
                        eprintln!("{} Failed to save state: {e}", "❌".red());
                        return;
                    }
                    println!(
                        "{} System RESUMED (Normal semester operations active).",
                        "▶️".green().bold()
                    );
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
                        Ok(s) if s.success() => println!(
                            "{} Sync service launched in background.",
                            "✅".green().bold()
                        ),
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
                        println!(
                            "⏱️ Weekly sync timer: {} (Runs Saturday 20:00:00)",
                            "ACTIVE".green().bold()
                        );
                    } else {
                        println!(
                            "⏱️ Weekly sync timer: {}",
                            "INACTIVE / PAUSED".yellow().bold()
                        );
                    }
                    println!(
                        "   Commands: '{}', '{}', '{}'",
                        "timer pause".cyan(),
                        "timer resume".cyan(),
                        "timer run".cyan()
                    );
                }
            }
            return;
        }
        Some(Commands::SetGist { id }) => {
            let clean_id = id.trim().to_string();
            match store.update(|s| s.gist_id = Some(clean_id.clone())) {
                Ok(()) => {
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
                Err(e) => {
                    eprintln!("{} Failed to save state: {e}", "❌".red());
                }
            }
            return;
        }
        Some(Commands::SyncUcam {
            force,
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
            let target_json = resolve_write_target(cli.file.as_ref());

            match academic_routine::run_ucam_scraper_opts_force(&target_json, *headed, *force) {
                Ok(path) => {
                    println!(
                        "{} Successfully extracted routine from UCAM!",
                        "✅".green().bold()
                    );
                    println!("   • Saved to: {}", path.display().to_string().cyan());

                    if let Ok(raw_sched) = load_routine_file(&path) {
                        println!("   • Total classes extracted: {}", raw_sched.entries.len());
                        println!("   • Semester: {}", raw_sched.semester_name.yellow().bold());

                        // Auto-import attendance records if attendance_data.json was extracted (M-08)
                        let att_path = path
                            .parent()
                            .unwrap_or_else(|| std::path::Path::new(""))
                            .join("attendance_data.json");
                        if att_path.exists() {
                            let admin_chat_id = resolve_admin_chat_id().unwrap_or(0);
                            if admin_chat_id > 0 {
                                match store.update(|s| {
                                    s.import_ucam_attendance_file(admin_chat_id, &att_path)
                                }) {
                                    Ok(Ok(count)) => {
                                        println!(
                                            "   • {} Imported official attendance for {} courses into Chat ID {}!",
                                            "📊".green().bold(),
                                            count,
                                            admin_chat_id
                                        );
                                    }
                                    Ok(Err(e)) => {
                                        eprintln!("   • ⚠️ Attendance import notice: {e}")
                                    }
                                    Err(e) => eprintln!("   • ⚠️ State save failed: {e}"),
                                }
                            } else {
                                println!("   • ℹ️ Attendance data extracted, but no ADMIN_CHAT_ID or TELEGRAM_CHAT_ID configured to assign attendance to.");
                            }
                        }

                        // Re-read fresh state after scrape & attendance import
                        let state = store.read();
                        let active_sched = raw_sched.filter_active(&state.dropped_courses);
                        let ics_path = resolve_ics_path();
                        let _ = export_ics_file(&active_sched, &ics_path, state.semester_end_date);
                        println!(
                            "   • Exported active calendar to: {}",
                            ics_path.display().to_string().cyan()
                        );

                        if *local_only {
                            println!("🔒 Local-only mode active: skipping cloud Gist and Google Sheets sync.");
                            return;
                        }

                        // Gist sync
                        if !*no_gist && state.auto_sync_gist {
                            if let Some(ref gid) = state.gist_id {
                                println!("🔄 Syncing routine data, calendar, and metadata to GitHub Gist...");
                                match academic_routine::sync_gist_bundle(
                                    gid,
                                    &ics_path,
                                    &path,
                                    &state.public_meta(),
                                )
                                .await
                                {
                                    Ok(url) => {
                                        println!(
                                            "🚀 Live subscription feed updated: {}",
                                            url.yellow().bold()
                                        );
                                        println!("   • Synced routine.ics, routine_data.json, and routine_meta.json to Gist!");
                                    }
                                    Err(e) => eprintln!("⚠️ Gist sync notice: {e}"),
                                }
                            }
                        } else if *no_gist {
                            println!("ℹ️ Gist sync skipped (--no-gist).");
                        }

                        // Google Sheets sync
                        if !*no_gsheet && state.auto_sync_gsheet {
                            let service_key_candidates =
                                [PathBuf::from("google_cloud_keys/service_account_key.json")];
                            let has_service_key = std::env::var("GOOGLE_SERVICE_ACCOUNT_KEY")
                                .map(PathBuf::from)
                                .map(|p| p.exists())
                                .unwrap_or(false)
                                || service_key_candidates.iter().any(|p| p.exists());

                            if has_service_key {
                                let sheet_name = std::env::var("SPREADSHEET_NAME")
                                    .unwrap_or_else(|_| "CSE-03_B_ClassRoutine".to_string());
                                println!(
                                    "📊 Syncing routine to Google Spreadsheet ({sheet_name})..."
                                );
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
            let target_json = match file.clone() {
                Some(f) => {
                    if !f.exists() {
                        eprintln!(
                            "{} Specified routine file does not exist: {}",
                            "❌".red().bold(),
                            f.display()
                        );
                        std::process::exit(1);
                    }
                    f
                }
                None => match resolve_routine_path(cli.file.as_ref()) {
                    Ok(Some(p)) => p,
                    Ok(None) => PathBuf::from("output/routine_data.json"),
                    Err(e) => {
                        eprintln!("{} {e}", "❌".red().bold());
                        std::process::exit(1);
                    }
                },
            };

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
            let routine_path = match resolve_routine_path(cli.file.as_ref()) {
                Ok(Some(p)) => p,
                Ok(None) => PathBuf::from("output/routine_data.json"),
                Err(e) => {
                    eprintln!("{} {e}", "❌".red().bold());
                    std::process::exit(1);
                }
            };

            let default_sec = cli
                .section
                .or_else(|| std::env::var("DEFAULT_SECTION").ok());

            let ctx = match academic_routine::BotContext::new(
                routine_path,
                state_file,
                default_sec,
                *no_http,
            ) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("{} Failed to initialize bot state store: {e}", "❌".red());
                    std::process::exit(1);
                }
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
        Ok(Some(p)) => p,
        Ok(None) => {
            eprintln!(
                "{} No routine JSON file found. Specify one with {} or place it at {}",
                "❌".red().bold(),
                "--file <PATH>".cyan(),
                "final_combined_routine.json".yellow()
            );
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("{} {e}", "❌".red().bold());
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
    let state = store.read();
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
        Some(Commands::Teacher { query }) => {
            let q = query.as_deref().unwrap_or("").trim();
            if q.is_empty() {
                println!("🔍 Usage: academic-routine teacher <name, initials, or course code>");
                println!("   Example: academic-routine teacher basar");
                println!("            academic-routine teacher SS");
                println!("            academic-routine teacher 4125\n");
                let dir = schedule.teacher_directory();
                println!("📚 All Faculty Members ({} total):", dir.len());
                for name in dir.keys() {
                    println!("   • {name}");
                }
                return;
            }

            let matches = schedule.find_teachers(q);
            if matches.is_empty() {
                println!(
                    "{} No faculty members found matching '{}'.",
                    "🔍".yellow(),
                    q.cyan()
                );
            } else {
                for (name, contact, courses) in &matches {
                    print!("{}", render_teacher_card(name, contact, courses));
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
                    println!(
                        "   • Active classes exported: {}",
                        sched_to_export.entries.len()
                    );
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

            let ics_path = resolve_ics_path();
            if !ics_path.exists() {
                let _ = export_ics_file(&schedule, &ics_path, state.semester_end_date);
            }

            println!(
                "{} Syncing routine, calendar, and metadata to GitHub Gist...",
                "🔄".cyan()
            );
            let json_path = match resolve_routine_path(cli.file.as_ref()) {
                Ok(Some(p)) => p,
                Ok(None) => PathBuf::from("output/routine_data.json"),
                Err(e) => {
                    eprintln!("{} {e}", "❌".red().bold());
                    std::process::exit(1);
                }
            };
            let result = if json_path.exists() {
                academic_routine::sync_gist_bundle(gid, &ics_path, &json_path, &state.public_meta())
                    .await
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
