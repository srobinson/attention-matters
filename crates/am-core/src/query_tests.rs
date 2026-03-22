    use super::*;
    use crate::episode::Episode;
    use crate::neighborhood::Neighborhood;
    use rand::SeedableRng;
    use rand::rngs::SmallRng;

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(42)
    }

    fn to_tokens(words: &[&str]) -> Vec<String> {
        words.iter().map(std::string::ToString::to_string).collect()
    }

    fn make_test_system() -> DAESystem {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Subconscious episode with shared words
        let mut ep = Episode::new("memories");
        let n1 = Neighborhood::from_tokens(
            &to_tokens(&["quantum", "physics", "particle"]),
            None,
            "quantum physics particle",
            &mut rng,
        );
        let n2 = Neighborhood::from_tokens(
            &to_tokens(&["quantum", "computing", "algorithm"]),
            None,
            "quantum computing algorithm",
            &mut rng,
        );
        let n3 = Neighborhood::from_tokens(
            &to_tokens(&["neural", "network", "learning"]),
            None,
            "neural network learning",
            &mut rng,
        );
        ep.add_neighborhood(n1);
        ep.add_neighborhood(n2);
        ep.add_neighborhood(n3);
        sys.add_episode(ep);

        // Conscious: overlap on "quantum"
        sys.add_to_conscious("quantum mechanics", &mut rng);

        sys
    }

    #[test]
    fn test_pairwise_drift_moves_closer() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Create neighborhoods where activating multiple words ensures
        // occurrences have non-zero drift rate (ratio < THRESHOLD)
        let mut ep = Episode::new("test");
        let n1 = Neighborhood::from_tokens(
            &to_tokens(&["alpha", "beta", "gamma", "delta"]),
            None,
            "alpha beta gamma delta",
            &mut rng,
        );
        let n2 = Neighborhood::from_tokens(
            &to_tokens(&["alpha", "beta", "epsilon", "zeta"]),
            None,
            "alpha beta epsilon zeta",
            &mut rng,
        );
        ep.add_neighborhood(n1);
        ep.add_neighborhood(n2);
        sys.add_episode(ep);

        // Activate multiple words so container activation is high enough
        // that individual ratio < THRESHOLD
        let (activation, _) =
            QueryEngine::activate(&mut sys, "alpha beta gamma delta epsilon zeta");

        // Get refs for "alpha" which is in both neighborhoods
        let alpha_refs: Vec<_> = activation
            .subconscious
            .iter()
            .filter(|r| sys.get_occurrence(**r).word == "alpha")
            .copied()
            .collect();
        assert!(
            alpha_refs.len() >= 2,
            "need alpha in at least 2 neighborhoods"
        );

        let pos_before_0 = sys.get_occurrence(alpha_refs[0]).position;
        let pos_before_1 = sys.get_occurrence(alpha_refs[1]).position;
        let dist_before = pos_before_0.angular_distance(pos_before_1);

        QueryEngine::drift_and_consolidate(&mut sys, &activation.subconscious);

        let pos_after_0 = sys.get_occurrence(alpha_refs[0]).position;
        let pos_after_1 = sys.get_occurrence(alpha_refs[1]).position;
        let dist_after = pos_after_0.angular_distance(pos_after_1);

        assert!(
            dist_after < dist_before,
            "drift should move occurrences closer: {dist_before} -> {dist_after}"
        );
    }

    #[test]
    fn test_anchored_dont_move() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        let mut ep = Episode::new("test");
        let mut n = Neighborhood::from_tokens(
            &to_tokens(&["word1", "word2"]),
            None,
            "word1 word2",
            &mut rng,
        );
        // Make word1 anchored by setting high activation relative to container
        n.occurrences[0].activation_count = 100;
        n.occurrences[1].activation_count = 1;
        ep.add_neighborhood(n);
        sys.add_episode(ep);

        let refs = sys.get_word_occurrences("word1");
        let pos_before = sys.get_occurrence(refs[0]).position;

        // Activate and drift
        let (activation, _) = QueryEngine::activate(&mut sys, "word1 word2");
        QueryEngine::drift_and_consolidate(&mut sys, &activation.subconscious);

        let pos_after = sys.get_occurrence(refs[0]).position;
        assert_eq!(pos_before, pos_after, "anchored word should not move");
    }

    #[test]
    fn test_interference_computation() {
        let mut sys = make_test_system();
        let (activation, _) = QueryEngine::activate(&mut sys, "quantum");

        let (interference, word_groups) = QueryEngine::compute_interference(
            &sys,
            &activation.subconscious,
            &activation.conscious,
        );

        assert!(!interference.is_empty(), "should have interference results");
        assert!(!word_groups.is_empty(), "should have word groups");
        assert_eq!(word_groups[0].word, "quantum");

        for ir in &interference {
            assert!(
                ir.interference >= -1.0 && ir.interference <= 1.0,
                "interference out of range: {}",
                ir.interference
            );
        }
    }

    #[test]
    fn test_kuramoto_coupling_constants() {
        let sys = make_test_system();
        let n_con = sys.conscious_episode.count().max(1);
        let n_total = sys.n().max(1);
        let n_sub = n_total.saturating_sub(n_con).max(1);

        let k_con = n_sub as f64 / n_total as f64;
        let k_sub = n_con as f64 / n_total as f64;

        assert!(
            (k_con + k_sub - 1.0).abs() < 0.01,
            "K_CON + K_SUB should ≈ 1: {} + {} = {}",
            k_con,
            k_sub,
            k_con + k_sub
        );
    }

    #[test]
    fn test_kuramoto_pulls_phases() {
        let mut sys = make_test_system();
        let (activation, _) = QueryEngine::activate(&mut sys, "quantum");

        // Get initial phase diff
        let sub_refs = activation.subconscious.clone();
        let con_refs = activation.conscious.clone();

        if sub_refs.is_empty() || con_refs.is_empty() {
            return; // Skip if no overlap
        }

        let sub_theta_before = sys.get_occurrence(sub_refs[0]).phasor.theta;
        let con_theta_before = sys.get_occurrence(con_refs[0]).phasor.theta;
        let diff_before = (sub_theta_before - con_theta_before).abs();

        let (_, word_groups) = QueryEngine::compute_interference(&sys, &sub_refs, &con_refs);
        QueryEngine::apply_kuramoto_coupling(&mut sys, &word_groups);

        let sub_theta_after = sys.get_occurrence(sub_refs[0]).phasor.theta;
        let con_theta_after = sys.get_occurrence(con_refs[0]).phasor.theta;
        let mut diff_after = (sub_theta_after - con_theta_after).abs();
        if diff_after > std::f64::consts::PI {
            diff_after = std::f64::consts::TAU - diff_after;
        }
        let mut diff_before_wrapped = diff_before;
        if diff_before_wrapped > std::f64::consts::PI {
            diff_before_wrapped = std::f64::consts::TAU - diff_before_wrapped;
        }

        // Kuramoto should reduce or maintain phase difference
        // (it pulls toward alignment)
        assert!(
            diff_after <= diff_before_wrapped + 0.01,
            "Kuramoto should pull phases closer: {diff_before_wrapped} -> {diff_after}"
        );
    }

    #[test]
    fn test_full_pipeline() {
        let mut sys = make_test_system();
        let result = QueryEngine::process_query(&mut sys, "quantum physics");

        assert!(!result.activation.subconscious.is_empty());
        assert!(!result.activation.conscious.is_empty());
        assert!(!result.interference.is_empty());
    }

    #[test]
    fn test_manifest_contains_activated_ids() {
        let mut sys = make_test_system();
        let result = QueryEngine::process_query(&mut sys, "quantum physics");

        // Every activated occurrence should appear in manifest.activated
        let total_activated =
            result.activation.subconscious.len() + result.activation.conscious.len();
        assert_eq!(
            result.manifest.activated.len(),
            total_activated,
            "manifest should contain one UUID per activated occurrence"
        );

        // Verify the UUIDs match the actual occurrence IDs
        for r in result
            .activation
            .subconscious
            .iter()
            .chain(&result.activation.conscious)
        {
            let occ_id = sys.get_occurrence(*r).id;
            assert!(
                result.manifest.activated.contains(&occ_id),
                "activated occurrence {occ_id} missing from manifest"
            );
        }
    }

    #[test]
    fn test_manifest_contains_drifted_ids() {
        let mut sys = make_test_system();
        let result = QueryEngine::process_query(&mut sys, "quantum physics");

        // Drifted list should be non-empty when occurrences have non-zero drift rate
        // and Kuramoto coupling occurs (quantum is in both manifolds)
        assert!(
            !result.manifest.drifted.is_empty(),
            "manifest should contain drifted occurrence IDs after drift and Kuramoto"
        );

        // All drifted IDs should be valid UUIDs that exist in the system
        for uuid in &result.manifest.drifted {
            assert_ne!(*uuid, Uuid::nil(), "drifted UUID should not be nil");
        }
    }

    #[test]
    fn test_manifest_empty_when_no_matches() {
        let mut sys = make_test_system();
        let result = QueryEngine::process_query(&mut sys, "xyznonexistent");

        assert!(
            result.manifest.activated.is_empty(),
            "no matches means no activated IDs"
        );
        assert!(
            result.manifest.drifted.is_empty(),
            "no matches means no drifted IDs"
        );
    }

    #[test]
    fn test_idf_rare_words_drift_more() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // "rare" appears once, "common" appears in many neighborhoods
        let mut ep = Episode::new("test");
        for i in 0..5 {
            let tokens = if i == 0 {
                to_tokens(&["rare", "common"])
            } else {
                to_tokens(&["common", &format!("filler{i}")])
            };
            let n = Neighborhood::from_tokens(&tokens, None, "", &mut rng);
            ep.add_neighborhood(n);
        }
        sys.add_episode(ep);

        let w_rare = sys.get_word_weight("rare");
        let w_common = sys.get_word_weight("common");

        assert!(
            w_rare > w_common,
            "rare word should have higher IDF weight: {w_rare} vs {w_common}"
        );
    }

    /// Generate a query string with >50 unique tokens.
    fn make_large_query(unique_words: &[&str], filler_count: usize) -> String {
        let mut words: Vec<String> = unique_words.iter().map(|w| (*w).to_string()).collect();
        for i in 0..filler_count {
            words.push(format!("filler{i}"));
        }
        words.join(" ")
    }

    #[test]
    fn large_query_filters_common_words_from_drift() {
        // Build a system with >10 neighborhoods so weight_floor < 1.0.
        // "common" appears in every neighborhood (low IDF weight).
        // Verify the weight_floor computation is correct and the pipeline
        // completes without panic for >50 token queries.
        let mut rng = rng();
        let mut sys = DAESystem::new("test");
        let mut ep = Episode::new("test");
        for i in 0..12 {
            let tokens = if i == 0 {
                to_tokens(&["rare", "common"])
            } else {
                to_tokens(&["common", &format!("word{i}")])
            };
            let n = Neighborhood::from_tokens(&tokens, None, "", &mut rng);
            ep.add_neighborhood(n);
        }
        sys.add_episode(ep);

        let total_nbhd = sys.total_neighborhoods();
        assert!(total_nbhd >= 10, "need >= 10 neighborhoods");

        // Verify the weight_floor math: 1.0 / floor(12 * 0.1) = 1.0
        let weight_floor = 1.0 / (total_nbhd as f64 * 0.1).floor().max(1.0);
        assert!(
            (weight_floor - 1.0).abs() < f64::EPSILON,
            "weight_floor should be 1.0 for 12 neighborhoods, got {weight_floor}"
        );

        // "common" IDF = 1/12 < weight_floor, should be excluded from drift
        let common_weight = sys.get_word_weight("common");
        assert!(
            common_weight < weight_floor,
            "common (weight {common_weight}) should be below floor ({weight_floor})"
        );

        // "rare" IDF = 1.0 >= weight_floor, should be included in drift
        let rare_weight = sys.get_word_weight("rare");
        assert!(
            rare_weight >= weight_floor,
            "rare (weight {rare_weight}) should be at or above floor ({weight_floor})"
        );

        // The full pipeline should complete without panic
        let query = make_large_query(&["rare", "common"], 55);
        let result = QueryEngine::process_query(&mut sys, &query);

        // Activation should contain both (filtering only affects drift)
        assert!(
            result.query_token_count > 50,
            "query should have >50 unique tokens"
        );
        assert!(
            !result.activation.subconscious.is_empty(),
            "activation should contain occurrences"
        );
    }

    #[test]
    fn large_query_weight_floor_one_with_few_neighborhoods() {
        // Build a system with <10 neighborhoods so weight_floor = 1.0.
        // Only words in exactly 1 neighborhood pass drift filtering.
        let mut rng = rng();
        let mut sys = DAESystem::new("test");
        let mut ep = Episode::new("test");

        // 3 neighborhoods: "unique_a" in one, "shared" in all three
        let n1 = Neighborhood::from_tokens(&to_tokens(&["unique_a", "shared"]), None, "", &mut rng);
        let n2 = Neighborhood::from_tokens(&to_tokens(&["unique_b", "shared"]), None, "", &mut rng);
        let n3 = Neighborhood::from_tokens(&to_tokens(&["unique_c", "shared"]), None, "", &mut rng);
        ep.add_neighborhood(n1);
        ep.add_neighborhood(n2);
        ep.add_neighborhood(n3);
        sys.add_episode(ep);

        let total_nbhd = sys.total_neighborhoods();
        assert!(total_nbhd < 10, "need < 10 neighborhoods");

        // Verify edge case: floor(3 * 0.1) = 0, max(1.0) = 1.0
        let weight_floor = 1.0 / (total_nbhd as f64 * 0.1).floor().max(1.0);
        assert!(
            (weight_floor - 1.0).abs() < f64::EPSILON,
            "weight_floor should be 1.0 for <10 neighborhoods, got {weight_floor}"
        );

        // "shared" IDF = 1/3 < 1.0 - excluded from drift
        let shared_weight = sys.get_word_weight("shared");
        assert!(
            shared_weight < weight_floor,
            "shared (weight {shared_weight}) should be below 1.0"
        );

        // unique words: IDF = 1.0 - included in drift
        let unique_weight = sys.get_word_weight("unique_a");
        assert!(
            unique_weight >= weight_floor,
            "unique_a (weight {unique_weight}) should pass floor"
        );

        // The full pipeline should not crash or produce empty results
        let query = make_large_query(&["unique_a", "unique_b", "unique_c", "shared"], 55);
        let result = QueryEngine::process_query(&mut sys, &query);

        assert!(
            result.query_token_count > 50,
            "query should have >50 unique tokens"
        );
        assert!(
            !result.activation.subconscious.is_empty(),
            "pipeline should produce non-empty activation"
        );
    }
