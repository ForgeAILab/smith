// provider behavior tests.

    #[test]
    fn built_in_defaults_claim_nothing_about_a_model() {
        let defaults = built_in_defaults(Path::new("/user/.smith"));
        assert!(defaults.models.is_empty());
        let context = defaults.context.expect("a context table");
        assert!(context.output_reserve.is_none());
        assert!(context.capability_budget.is_none());
        assert_eq!(context.reasoning_reserve, Some(0));
    }
