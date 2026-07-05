use super::*;

#[test]
fn test_date_decomposition() {
    let dt = decompose_timestamp(1719000000);
    assert_eq!(dt.year, 2024);
    assert_eq!(dt.month, 6);
    assert_eq!(dt.day_of_month, 21);
    assert_eq!(dt.hour, 20);
    assert_eq!(dt.minute, 0);
    assert_eq!(dt.day_of_week, 5);
}

#[test]
fn test_cron_matching() {
    let dt = DateTimeDecomposed {
        year: 2026,
        month: 6,
        day_of_month: 22,
        hour: 12,
        minute: 30,
        day_of_week: 1,
    };

    assert!(match_cron("* * * * *", &dt));
    assert!(match_cron("30 12 * * *", &dt));
    assert!(match_cron("*/5 */6 * * 1", &dt));
    assert!(match_cron("20,30,40 10-15 * * 1-5", &dt));

    assert!(!match_cron("0 * * * *", &dt));
    assert!(!match_cron("30 10 * * *", &dt));
    assert!(!match_cron("* * * * 0", &dt));
}

#[test]
fn test_calculate_next_run() {
    let start = 1782216000;
    let next = calculate_next_run("30 12 * * *", start);
    assert!(next.is_some());
    let dt = decompose_timestamp(next.unwrap());
    assert_eq!(dt.hour, 12);
    assert_eq!(dt.minute, 30);
}
