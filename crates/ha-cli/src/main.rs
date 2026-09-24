//! Home Advisor CLI: the milestone-1 proof-of-life and test harness.
//!
//! The finished CLI seeds a demo family, builds a purpose-limited outbound
//! research context, runs it through the privacy gate, prints the privacy
//! receipt, and writes the verdict to the egress log. This scaffold prints a
//! banner so the workspace round trip is runnable from day one.

fn banner() -> String {
    format!("Home Advisor CLI (scaffold) v{}", ha_core::VERSION)
}

fn main() {
    println!("{}", banner());
}

#[cfg(test)]
mod tests {
    use super::banner;

    #[test]
    fn banner_names_the_project() {
        assert!(banner().contains("Home Advisor CLI"));
    }
}
