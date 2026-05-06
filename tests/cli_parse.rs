use logit::cli::parse_cli;
use logit::domain::{AliasCommand, DomainCommand};

#[test]
fn alias_named_cache_can_be_set() {
    let parsed = parse_cli(vec![
        String::from("alias"),
        String::from("cache"),
        String::from("TC-1234"),
    ])
    .expect("alias parses");

    match parsed.command {
        DomainCommand::Alias(AliasCommand::Set(input)) => {
            assert_eq!(input.name, "cache");
            assert_eq!(input.issue_key, "TC-1234");
        }
        other => panic!("unexpected command: {other:?}"),
    }
}
