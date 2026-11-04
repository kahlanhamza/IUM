//! The multi-path scheduling plan, pure-fn tested BEFORE the module exists
//! (issue #51, RED-first — the tests below failed to compile on the clean
//! tree, which was the RED run).
//!
//! What is pinned here, no wire attached:
//!
//! * the capability truth table — which `NetPath` bind modes this OS
//!   declares (ADR-011 "adapter declares, planner obeys" extended to
//!   bindings);
//! * discovery's filter — operstate up, `lo` excluded by name, DOWN
//!   interfaces invisible, the user override list wins verbatim, and an
//!   empty discovery DEGRADES to the single default path (never a job
//!   failure);
//! * the weighted round-robin — weights ∝ measured bps, largest-remainder
//!   so a 3:1 split is deterministic, a 0-bps path is EXCLUDED (the
//!   prober's per-path verdict);
//! * the per-path AIMD step — additive increase on a clean piece,
//!   multiplicative decrease on an error spike, floored at 1 so a
//!   struggling path starves but never vanishes (exclusion is the prober's
//!   job, not the scheduler's);
//! * the tus composition rule — N partials per path × P paths, inside the
//!   #50 capability ceiling.

use ium_core::netpath::{self, NetPath, PathPlan};

// ---------------------------------------------------------------------------
// capability truth table (compile-time facts, asserted as runtime truth)
// ---------------------------------------------------------------------------

#[test]
fn default_path_always_declares() {
    let n = NetPath::Default;
    assert!(netpath::bind_supported(&n) || matches!(n, NetPath::Default));
    // Default binds nothing; it is supported on every OS by construction.
    assert!(netpath::bind_supported(&NetPath::Default));
}

#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "android",
    target_os = "fuchsia",
    target_os = "solaris",
    target_os = "illumos"
))]
#[test]
fn bind_device_declares_on_this_os() {
    // W1: reqwest's cfg gate covers this OS, so BindDevice is declarable.
    assert!(netpath::bind_supported(&NetPath::BindDevice("eth0".into())));
}

#[cfg(target_os = "windows")]
#[test]
fn bind_device_loudly_unavailable_on_windows() {
    // The honest loud-skip: the variant EXISTS (a plan can carry it), but
    // building a client for it is a loud error, never a silent fallback.
    assert!(!netpath::bind_supported(&NetPath::BindDevice(
        "eth0".into()
    )));
    assert!(netpath::client_builder(&NetPath::BindDevice("eth0".into())).is_err());
}

#[test]
fn source_addr_declares_everywhere() {
    use std::net::Ipv4Addr;
    assert!(netpath::bind_supported(&NetPath::SourceAddr(
        Ipv4Addr::new(127, 0, 0, 1).into()
    )));
}

// ---------------------------------------------------------------------------
// discovery filter (injected sysfs — no /sys in unit tests)
// ---------------------------------------------------------------------------

#[test]
fn discovery_filters_operstate_and_loopback() {
    let sysfs = vec![
        ("lo", "unknown"),  // loopback: excluded BY NAME, whatever its state
        ("eth0", "up"),     // real uplink: in
        ("dummy0", "down"), // DOWN: invisible
        ("wlan0", "up"),    // second uplink: in
    ];
    let paths = netpath::discover_from(&sysfs, None).expect("discovery");
    let names = netpath::path_names(&paths);
    assert_eq!(names, vec!["eth0".to_string(), "wlan0".to_string()]);
}

#[test]
fn discovery_override_wins_verbatim() {
    let sysfs = vec![("eth0", "up"), ("wlan0", "up")];
    let override_list = vec!["eth9".to_string()]; // not even in sysfs
    let paths = netpath::discover_from(&sysfs, Some(&override_list)).expect("override");
    // The user's list is obeyed verbatim (BindDevice carries the name even
    // if the iface does not exist YET — connecting fails loudly then).
    assert_eq!(netpath::path_names(&paths), vec!["eth9".to_string()]);
}

#[test]
fn discovery_degrades_to_default_never_fails() {
    // Nothing UP at all (a container with no ifaces): the job still runs,
    // on the single default path — degradation, not failure.
    let sysfs = vec![("lo", "unknown"), ("dummy0", "down")];
    let paths = netpath::discover_from(&sysfs, None).expect("discovery");
    assert_eq!(paths, vec![NetPath::Default]);
    // And a total sysfs read failure is the same degradation.
    let paths = netpath::discover_from(&[] as &[(&str, &str)], None).expect("discovery");
    assert_eq!(paths, vec![NetPath::Default]);
}

// ---------------------------------------------------------------------------
// weighted round-robin (pure)
// ---------------------------------------------------------------------------

fn plan_with(bps: &[f64]) -> PathPlan {
    let paths: Vec<NetPath> = (0..bps.len())
        .map(|i| {
            if i == 0 {
                NetPath::Default
            } else {
                NetPath::SourceAddr(std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)))
            }
        })
        .collect();
    netpath::plan_weights(&paths, bps).expect("plan")
}

#[test]
fn weights_track_measured_bps() {
    // 3 paths at 3:2:1 MiB/s → weights 3:2:1 (bps is the whole signal).
    let plan = plan_with(&[3_000_000.0, 2_000_000.0, 1_000_000.0]);
    assert_eq!(plan.weights, vec![3.0, 2.0, 1.0]);
}

#[test]
fn zero_bps_path_is_excluded() {
    // The prober's verdict on a dead path: excluded from the plan, named in
    // the journal payload — the scheduler never assigns to it.
    let plan = plan_with(&[3_000_000.0, 0.0, 1_000_000.0]);
    assert_eq!(plan.weights.len(), 2);
    assert_eq!(plan.excluded, vec![1]);
}

#[test]
fn all_paths_dead_degrades_to_default() {
    // Every path measured 0: the plan falls back to the single default
    // path (degradation, never a job failure — the issue's rule).
    let plan = plan_with(&[0.0, 0.0]);
    assert_eq!(plan.weights, vec![1.0]);
    assert_eq!(plan.excluded, vec![0, 1]);
    assert_eq!(plan.assignment_paths(), &[NetPath::Default]);
}

#[test]
fn largest_remainder_split_is_deterministic() {
    // 7 pieces across a 3:1 pair → 3:1 split of 7 = 5.25:1.75 → the
    // largest-remainder fix lands 5 and 2, EVERY time (no float drift, no
    // round-robin wobble).
    let plan = plan_with(&[3.0, 1.0]);
    let counts = netpath::assign(&plan, 7);
    assert_eq!(counts, vec![5, 2]);
    // 1 piece on the same pair: the fastest path takes it.
    assert_eq!(netpath::assign(&plan, 1), vec![1, 0]);
}

#[test]
fn single_path_plan_assigns_everything() {
    let plan = plan_with(&[10.0]);
    assert_eq!(netpath::assign(&plan, 13), vec![13]);
}

// ---------------------------------------------------------------------------
// per-path AIMD (the outer loop over #43's per-session realign)
// ---------------------------------------------------------------------------

#[test]
fn aimd_increases_on_clean_pieces() {
    let w = netpath::step_weight(1.0, netpath::Signal::Clean);
    assert!(w > 1.0, "clean piece must additively increase");
}

#[test]
fn aimd_decreases_on_error_spike() {
    let w = netpath::step_weight(4.0, netpath::Signal::ErrorSpike);
    assert!(w < 4.0, "error spike must multiplicatively decrease");
}

#[test]
fn aimd_floor_is_one_never_zero() {
    // A path that keeps erroring starves toward 1 but NEVER reaches 0:
    // zeroing is the prober's exclusion call, not a scheduler side effect.
    let mut w = 8.0;
    for _ in 0..64 {
        w = netpath::step_weight(w, netpath::Signal::ErrorSpike);
    }
    assert!((w - 1.0).abs() < 1e-9, "floor at 1, got {w}");
}

#[test]
fn aimd_ceiling_is_declared() {
    // The weight cannot outrun the path's measured share by more than the
    // declared ceiling — a fast path borrowing slack must not starve the
    // fleet's honest slow path.
    let mut w = 1.0;
    for _ in 0..64 {
        w = netpath::step_weight(w, netpath::Signal::Clean);
    }
    assert!(w <= netpath::WEIGHT_CEILING, "ceiling breached: {w}");
}

// ---------------------------------------------------------------------------
// tus composition (#50 capability × #51 paths)
// ---------------------------------------------------------------------------

#[test]
fn concat_composes_paths_times_partials() {
    // #50 declared: up to N=4 partials per session-plan; #51 adds P=2
    // paths → the composed lane plans 8 partials, still per-path within
    // the declared ceiling.
    assert_eq!(netpath::compose_tus_partials(4, 2), 8);
    assert_eq!(netpath::compose_tus_partials(4, 1), 4);
    // No concat capability (N=1): one session per path, no multiplication.
    assert_eq!(netpath::compose_tus_partials(1, 3), 3);
}

#[test]
fn composition_never_exceeds_declared_total() {
    // The composed count is clamped by the lane's declared global ceiling —
    // composition multiplies within what the server/plan allows.
    assert_eq!(
        netpath::compose_tus_partials_clamped(4, 4, 8),
        8,
        "4 paths × 4 partials must clamp to the declared 8"
    );
}
