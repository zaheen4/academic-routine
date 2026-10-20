use academic_routine::DayOfWeek;

fn main() {
    println!("🎓 Academic Routine CLI v0.1.0");
    println!("Academic week days in university order:");
    for day in DayOfWeek::all() {
        println!(
            "  {}. {} ({}) [{}]",
            *day as u8,
            day.full_name(),
            day.as_str(),
            day.ical_code()
        );
    }
}
