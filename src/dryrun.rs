use chrono::NaiveDate;

use crate::domain::{Profile, WorklogDraft};
use crate::style::Style;

pub fn render_draft(
    draft: &WorklogDraft,
    profile_name: &str,
    profile: &Profile,
    today: NaiveDate,
    style: &Style,
    verbose: bool,
) -> String {
    crate::ui::render_dry_run(draft, profile_name, profile, today, style, verbose)
}
