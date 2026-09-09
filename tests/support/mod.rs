//! Expand the emitted RFC 5545 DTSTART + RDATE recurrence sets for assertions
//! about actual class occurrences. Keep structural assertions on the raw ICS.
#![allow(dead_code)]
use chrono::NaiveDateTime;

pub fn expand_calendar(ics: &str) -> String {
    let unfolded = ics.replace("\r\n ", "");
    let mut expanded = String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//test//EN\r\n");
    for block in unfolded.split("BEGIN:VEVENT\r\n").skip(1) {
        let body = block.split_once("END:VEVENT\r\n").unwrap().0;
        let value = |name: &str| body.lines().find_map(|l| l.strip_prefix(name)).unwrap();
        let parse = |s: &str| NaiveDateTime::parse_from_str(s, "%Y%m%dT%H%M%SZ").unwrap();
        let start = value("DTSTART:");
        let duration = parse(value("DTEND:")) - parse(start);
        let mut starts = vec![start];
        for dates in body.lines().filter_map(|l| l.strip_prefix("RDATE:")) {
            starts.extend(dates.split(','));
        }
        for occurrence in starts {
            expanded.push_str("BEGIN:VEVENT\r\n");
            expanded.push_str(&format!(
                "DTSTART:{occurrence}\r\nDTEND:{}\r\n",
                (parse(occurrence) + duration).format("%Y%m%dT%H%M%SZ")
            ));
            for line in body.lines().filter(|l| {
                !l.starts_with("DTSTART:") && !l.starts_with("DTEND:") && !l.starts_with("RDATE:")
            }) {
                expanded.push_str(line);
                expanded.push_str("\r\n");
            }
            expanded.push_str("END:VEVENT\r\n");
        }
    }
    expanded.push_str("END:VCALENDAR\r\n");
    expanded
}
