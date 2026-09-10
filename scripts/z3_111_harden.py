from pathlib import Path

path = Path("src/session_restore.rs")
text = path.read_text(encoding="utf-8")
marker = '''    #[test]\n    fn codec_round_trips_order_active_state_and_next_identities() {'''
block = r'''    #[test]
    fn count_bounds_are_enforced_without_mutating_past_limits() {
        let mut windows = SessionRestoreSnapshot::default();
        for _ in 0..MAX_SESSION_WINDOWS {
            windows.add_window().unwrap();
        }
        assert!(matches!(
            windows.add_window(),
            Err(SessionRestoreError::WindowLimitExceeded { .. })
        ));
        assert_eq!(windows.len(), MAX_SESSION_WINDOWS);

        let mut per_window = SessionRestoreSnapshot::default();
        let window = per_window.add_window().unwrap();
        for index in 0..MAX_SESSION_TABS_PER_WINDOW {
            per_window
                .add_tab(window, format!("https://tab-{index}.test"))
                .unwrap();
        }
        assert!(matches!(
            per_window.add_tab(window, "https://overflow.test"),
            Err(SessionRestoreError::TabsPerWindowLimitExceeded { .. })
        ));
        assert_eq!(per_window.total_tabs(), MAX_SESSION_TABS_PER_WINDOW);

        let mut total = SessionRestoreSnapshot::default();
        for window_index in 0..(MAX_SESSION_TABS / MAX_SESSION_TABS_PER_WINDOW) {
            let window = total.add_window().unwrap();
            for tab_index in 0..MAX_SESSION_TABS_PER_WINDOW {
                total
                    .add_tab(
                        window,
                        format!("https://total-{window_index}-{tab_index}.test"),
                    )
                    .unwrap();
            }
        }
        let overflow_window = total.add_window().unwrap();
        assert_eq!(total.total_tabs(), MAX_SESSION_TABS);
        assert!(matches!(
            total.add_tab(overflow_window, "https://total-overflow.test"),
            Err(SessionRestoreError::TabLimitExceeded { .. })
        ));
        assert!(total.window(overflow_window).unwrap().tabs().is_empty());
    }

    #[test]
    fn identity_exhaustion_is_failure_atomic() {
        let mut windows = SessionRestoreSnapshot::default();
        windows.next_window_id = u64::MAX;
        assert_eq!(
            windows.add_window(),
            Err(SessionRestoreError::WindowIdExhausted)
        );
        assert!(windows.is_empty());
        assert_eq!(windows.next_window_id, u64::MAX);

        let mut tabs = SessionRestoreSnapshot::default();
        let window = tabs.add_window().unwrap();
        tabs.next_tab_id = u64::MAX;
        assert_eq!(
            tabs.add_tab(window, "https://exhausted.test"),
            Err(SessionRestoreError::TabIdExhausted)
        );
        assert!(tabs.window(window).unwrap().tabs().is_empty());
        assert_eq!(tabs.window(window).unwrap().active_tab(), None);
        assert_eq!(tabs.next_tab_id, u64::MAX);
    }

'''
if text.count(marker) != 1:
    raise SystemExit("expected codec test marker exactly once")
text = text.replace(marker, block + marker, 1)

marker2 = '''    #[test]\n    fn retained_generations_remain_bounded() {'''
block2 = r'''    #[test]
    fn generation_discovery_and_all_corrupt_failure_are_bounded_and_explicit() {
        let corrupt_directory = TestDirectory::new();
        let corrupt_store = SessionRestoreStore::open(corrupt_directory.path()).unwrap();
        fs::write(corrupt_store.session_path(1), b"broken-one").unwrap();
        fs::write(corrupt_store.session_path(2), b"broken-two").unwrap();
        assert_eq!(
            corrupt_store.load(),
            Err(SessionRestoreError::NoValidGeneration {
                corrupt_generations: vec![2, 1],
            })
        );

        let bounded_directory = TestDirectory::new();
        let bounded_store = SessionRestoreStore::open(bounded_directory.path()).unwrap();
        for generation in 1..=(MAX_DISCOVERED_GENERATIONS as u64 + 1) {
            fs::write(bounded_store.session_path(generation), b"x").unwrap();
        }
        assert!(matches!(
            bounded_store.discover_generations(),
            Err(SessionRestoreError::GenerationFileLimitExceeded { found, limit })
                if found == MAX_DISCOVERED_GENERATIONS + 1
                    && limit == MAX_DISCOVERED_GENERATIONS
        ));
    }

'''
if text.count(marker2) != 1:
    raise SystemExit("expected retained generations marker exactly once")
path.write_text(text.replace(marker2, block2 + marker2, 1), encoding="utf-8")
