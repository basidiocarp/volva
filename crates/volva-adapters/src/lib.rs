#[must_use]
pub fn adapter_names() -> Vec<&'static str> {
    vec!["hyphae", "rhizome", "cortina", "canopy", "stipe"]
}

#[cfg(test)]
mod tests {
    use super::adapter_names;

    #[test]
    fn returns_expected_adapters() {
        assert_eq!(
            adapter_names(),
            vec!["hyphae", "rhizome", "cortina", "canopy", "stipe"]
        );
    }

    #[test]
    fn no_duplicates() {
        let names = adapter_names();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(names.len(), sorted.len());
    }

    #[test]
    fn all_names_non_empty() {
        for name in adapter_names() {
            assert!(!name.is_empty());
        }
    }
}
