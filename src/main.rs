use academic_routine::{load_routine_file, DayOfWeek};
use std::path::Path;

fn main() {
    println!("==================================================");
    println!("🎓 Academic Routine CLI v0.1.0");
    println!("==================================================");

    let default_path = Path::new(
        "/home/eyepatch/Documents/projects/routine-to-gsheet/output_of_fetched_routine/final_combined_routine.json",
    );

    let routine_path = if default_path.exists() {
        default_path
    } else {
        Path::new("final_combined_routine.json")
    };

    if !routine_path.exists() {
        println!("⚠️ No routine file found at {}", routine_path.display());
        return;
    }

    match load_routine_file(routine_path) {
        Ok(schedule) => {
            println!("📚 Semester : {}", schedule.semester_name);
            println!(
                "📊 Classes  : {} scheduled sessions",
                schedule.entries.len()
            );
            println!(
                "🎯 Courses  : {} unique active courses",
                schedule.all_course_codes().len()
            );

            let conflicts = schedule.find_conflicts();
            if conflicts.is_empty() {
                println!("✅ Conflicts: Zero scheduling conflicts detected!\n");
            } else {
                println!("⚠️ Conflicts: Found {} conflict(s)!\n", conflicts.len());
                for c in &conflicts {
                    println!("   - [{}] {}", c.day, c.reason);
                }
                println!();
            }

            for day in DayOfWeek::all() {
                let classes = schedule.for_day(*day);
                if classes.is_empty() {
                    continue;
                }

                println!("📅 {} ({} classes):", day.full_name(), classes.len());
                for c in classes {
                    let lab_tag = if c.is_lab() { " [LAB]" } else { "" };
                    println!(
                        "   • {:<22} | {:<9} | {:<30}{}",
                        c.time_slot.format_12h(),
                        format!("Room {}", c.room),
                        format!("{}: {}", c.code, c.title),
                        lab_tag
                    );
                    if !c.teacher.name.is_empty() {
                        println!("     └─ {}", c.teacher.formatted_summary());
                    }
                }
                println!();
            }
        }
        Err(e) => {
            eprintln!("❌ Failed to load routine: {e}");
        }
    }
}
