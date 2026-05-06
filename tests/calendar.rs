use chrono::NaiveDate;

#[test]
fn no_calendar_uses_configured_workday() {
    let profile = logit::config::default_profile("UTC");
    let context = logit::calendar::CalendarContext::configured(
        &profile,
        NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
        NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
    )
    .expect("configured context");

    assert!(matches!(
        context
            .day(NaiveDate::from_ymd_opt(2026, 4, 1).unwrap())
            .unwrap()
            .source,
        logit::domain::ExpectedTimeSource::ConfiguredWorkday
    ));
}
