//! Classifies observed syscall names against the real soma-jail policy tables.
//! Input: one syscall name per line on stdin. Output: name TAB verdict.
use std::io::Read;
use soma_jail::{NEVER_ALLOWED, syscall_rules};

fn verdict(name: &str) -> &'static str {
    if NEVER_ALLOWED.iter().any(|(n, _)| *n == name) {
        return "KILLED-always (documented denial surface)";
    }
    match syscall_rules().iter().find(|r| r.name == name) {
        Some(rule) if rule.steady.is_some() => "allowed-startup+steady",
        Some(_) => "allowed-startup-only (KILLED in steady state)",
        None => "KILLED-always (absent from the policy table)",
    }
}

fn main() {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).expect("stdin");
    for name in input.split_whitespace() {
        println!("{name}\t{}", verdict(name));
    }
}
