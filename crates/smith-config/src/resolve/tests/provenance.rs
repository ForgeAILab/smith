// provenance behavior tests.

    #[test]
    fn every_layer_has_a_distinct_rank() {
        let ranks: Vec<u8> = Layer::all()
            .iter()
            .map(|layer| layer.precedence())
            .collect();
        assert_eq!(ranks, (0..8).collect::<Vec<u8>>());
    }
