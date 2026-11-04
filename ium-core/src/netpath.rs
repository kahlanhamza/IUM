//! Multi-path bonding: the `NetPath` abstraction, discovery, and the
//! weighted scheduling plan (issue #51, M8#4).
//!
//! The chunk pool is the scheduler; this module supplies the OUTER loop it
//! rides: which physical paths exist, what each measured, and how pieces
//! spread across them. Everything here obeys two house rules:
//!
//! * **adapter declares, planner obeys** (ADR-011/012 extended to
//!   bindings): `bind_supported` is the declaration; the planner never
//!   invents a binding the OS+reqwest pair does not declare. The
//!   declaration comes from reqwest's own cfg surface (wire fact W1:
//!   `ClientBuilder::interface` is SO_BINDTODEVICE on Linux, IP_BOUND_IF
//!   on macOS, and cfg-gated to a fixed OS list that excludes Windows).
//! * **degradation, never failure**: an empty discovery, a dead path, a
//!   total sysfs outage — every one degrades toward today's behavior (the
//!   single default path). The type signatures carry the rule:
//!   `discover_from` cannot fail (`Infallible`), a zero-bps path is
//!   excluded and journaled, an all-dead plan falls back to `Default`.
//!
//! Wire facts (rehearsed live, `scripts/wirefacts51/`): source-address
//! binding needs no privilege (loopback carries 127.0.0.0/8 — the drill
//! rig; multi-NIC hosts give each uplink its own source IP);
//! SO_BINDTODEVICE succeeded unprivileged on the rehearsal sandbox but
//! MUST stay capability-declared, never assumed.

use std::convert::Infallible;
use std::net::IpAddr;

/// The reqwest cfg surface for `ClientBuilder::interface`, mirrored
/// verbatim (wire fact W1). Windows and the other non-listed targets
/// declare `BindDevice` unavailable — the loud-skip lane.
const INTERFACE_OS_SUPPORTED: bool = cfg!(any(
    target_os = "android",
    target_os = "fuchsia",
    target_os = "illumos",
    target_os = "ios",
    target_os = "linux",
    target_os = "macos",
    target_os = "solaris",
    target_os = "tvos",
    target_os = "visionos",
    target_os = "watchos",
));

/// One physical upload path.
///
/// `Default` binds nothing (today's behavior); `SourceAddr` fixes the
/// source IP (the no-privilege form — the drill rig and the multi-NIC
/// rule); `BindDevice` pins the interface name (the strong form,
/// OS-declared).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum NetPath {
    /// The system's default routing — no binding applied.
    Default,
    /// Bind every connection's source to this address.
    SourceAddr(IpAddr),
    /// Bind every connection to this interface name
    /// (SO_BINDTODEVICE / IP_BOUND_IF where the OS declares it).
    BindDevice(String),
}

/// The journal/name form of a path list (same order).
pub fn path_names(paths: &[NetPath]) -> Vec<String> {
    paths.iter().map(|p| p.name()).collect()
}

impl NetPath {
    /// The journal/name form of this path.
    pub fn name(&self) -> String {
        match self {
            NetPath::Default => "default".to_string(),
            NetPath::SourceAddr(ip) => ip.to_string(),
            NetPath::BindDevice(iface) => iface.clone(),
        }
    }
}

/// Whether THIS build's OS declares the binding a `NetPath` needs.
///
/// The ADR-011 declaration seam: the planner consults this before putting
/// a path into a plan; the probe consults it before spending a canary on
/// it.
pub fn bind_supported(path: &NetPath) -> bool {
    match path {
        NetPath::Default => true,
        NetPath::SourceAddr(_) => true,
        // `local_address` has no OS gate in reqwest — it is plain
        // source-address selection. `interface` is the gated one.
        NetPath::BindDevice(_) => INTERFACE_OS_SUPPORTED,
    }
}

/// A `reqwest::ClientBuilder` with the path's binding applied.
///
/// Loud failure (`PathError::BindUnavailable`) where the OS does not
/// declare the binding — never a silent fallback to `Default` (a silent
/// fallback would measure one link and journal another).
pub fn client_builder(path: &NetPath) -> Result<reqwest::ClientBuilder, PathError> {
    match path {
        NetPath::Default => Ok(reqwest::Client::builder()),
        NetPath::SourceAddr(ip) => Ok(reqwest::Client::builder().local_address(*ip)),
        NetPath::BindDevice(iface) => {
            // COMPILE-TIME cfg at the CALL SITE: on Windows (and the other
            // non-listed targets) `interface` does not EXIST — the method
            // itself is cfg-gated away in reqwest, so a runtime bool
            // cannot hide the call from the compiler (run #98,
            // core windows-latest: E0599 no method named `interface`).
            #[cfg(any(
                target_os = "android",
                target_os = "fuchsia",
                target_os = "illumos",
                target_os = "ios",
                target_os = "linux",
                target_os = "macos",
                target_os = "solaris",
                target_os = "tvos",
                target_os = "visionos",
                target_os = "watchos",
            ))]
            {
                Ok(reqwest::Client::builder().interface(iface))
            }
            #[cfg(not(any(
                target_os = "android",
                target_os = "fuchsia",
                target_os = "illumos",
                target_os = "ios",
                target_os = "linux",
                target_os = "macos",
                target_os = "solaris",
                target_os = "tvos",
                target_os = "visionos",
                target_os = "watchos",
            )))]
            {
                Err(PathError::BindUnavailable {
                    os: std::env::consts::OS.to_string(),
                    iface: iface.clone(),
                })
            }
        }
    }
}

/// Why a path's client could not be built.
#[derive(Debug, thiserror::Error)]
pub enum PathError {
    /// The OS does not declare the binding (the loud-skip lane).
    #[error("interface binding '{iface}' is unavailable on {os} (reqwest cfg gate)")]
    BindUnavailable {
        /// The interface that was asked for.
        iface: String,
        /// The OS that does not declare it.
        os: String,
    },
    /// The HTTP client itself refused to build.
    #[error("client build failed: {0}")]
    Client(#[from] reqwest::Error),
}

/// Build a ready `reqwest::Client` for the path.
pub fn client(path: &NetPath) -> Result<reqwest::Client, PathError> {
    Ok(client_builder(path)?.build()?)
}

/// Parse a user-supplied path spec (the JobSpec `paths` override list).
///
/// * `default` → [`NetPath::Default`]
/// * `addr:203.0.113.7` / a bare IP literal → [`NetPath::SourceAddr`]
/// * `iface:eth1` / any other string → [`NetPath::BindDevice`]
pub fn parse_spec(spec: &str) -> NetPath {
    if spec.eq_ignore_ascii_case("default") {
        return NetPath::Default;
    }
    if let Some(rest) = spec.strip_prefix("addr:") {
        if let Ok(ip) = rest.parse::<IpAddr>() {
            return NetPath::SourceAddr(ip);
        }
    }
    if let Ok(ip) = spec.parse::<IpAddr>() {
        return NetPath::SourceAddr(ip);
    }
    let iface = spec.strip_prefix("iface:").unwrap_or(spec);
    NetPath::BindDevice(iface.to_string())
}

/// One `(interface, operstate)` pair as `/sys/class/net` reports it.
pub type IfaceState = (String, String);

/// Discover candidate paths from the raw sysfs table.
///
/// Filter: operstate `up`, loopback excluded by name (`lo` reports
/// `unknown`, not `down` — the name rule is what actually filters it,
/// wire fact P1). The user override list wins verbatim (`parse_spec` on
/// each entry, no sysfs cross-check — a not-yet-existing interface fails
/// loudly at connect time, which is the honest order). An empty result
/// DEGRADES to `[NetPath::Default]` — the infallible signature carries
/// the issue's "never a job failure" rule.
pub fn discover_from<S1: AsRef<str>, S2: AsRef<str>>(
    sysfs: &[(S1, S2)],
    override_list: Option<&[String]>,
) -> Result<Vec<NetPath>, Infallible> {
    if let Some(list) = override_list {
        if !list.is_empty() {
            return Ok(list.iter().map(|s| parse_spec(s)).collect());
        }
    }
    let mut paths = Vec::new();
    for (name, state) in sysfs {
        let (name, state) = (name.as_ref(), state.as_ref());
        if name == "lo" || state != "up" {
            continue;
        }
        paths.push(NetPath::BindDevice(name.to_string()));
    }
    if paths.is_empty() {
        paths.push(NetPath::Default);
    }
    Ok(paths)
}

/// Read `/sys/class/net` and discover (the production entry).
pub fn discover(override_list: Option<&[String]>) -> Vec<NetPath> {
    let mut table = Vec::new();
    if let Ok(dir) = std::fs::read_dir("/sys/class/net") {
        for entry in dir.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let state = std::fs::read_to_string(entry.path().join("operstate"))
                .unwrap_or_default()
                .trim()
                .to_string();
            table.push((name, state));
        }
    }
    discover_from(&table, override_list).unwrap_or_else(|i| match i {})
}

/// The scheduler's plan: live paths, their weights, who was excluded.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PathPlan {
    /// Weight per LIVE path, compacted over the probed set with the
    /// excluded indexes removed (the minimum live weight is normalized
    /// to 1.0); `live_indexes` maps weights back to probed positions.
    pub weights: Vec<f64>,
    /// Indexes into the PROBED path list that measured 0 throughput —
    /// journaled as `path_excluded`, never assigned to.
    pub excluded: Vec<usize>,
    /// The probed path list this plan was built from (excluded indexes
    /// refer here; `assignment_paths` is the live view).
    pub probed: Vec<NetPath>,
}

impl PathPlan {
    /// Probed indexes of the LIVE paths — the positions `weights` lines
    /// up with. Empty when every probed path died (the degrade case).
    pub fn live_indexes(&self) -> Vec<usize> {
        (0..self.probed.len())
            .filter(|i| !self.excluded.contains(i))
            .collect()
    }

    /// The paths pieces actually assign to (live only; all-dead → the
    /// single default path).
    pub fn assignment_paths(&self) -> Vec<NetPath> {
        let live = self.live_indexes();
        if live.is_empty() {
            vec![NetPath::Default]
        } else {
            live.into_iter().map(|i| self.probed[i].clone()).collect()
        }
    }
}

/// Weight pieces across probed paths by measured throughput.
///
/// Weight ∝ bps, normalized so the slowest LIVE path weighs 1.0 (a 3:2:1
/// MiB/s trio plans 3:2:1). A 0-bps path is excluded (the prober's
/// verdict) and named in `excluded`. When every probed path is dead the
/// plan degrades: one weight, the default path — the job proceeds the
/// way it would have without bonding.
pub fn plan_weights(probed: &[NetPath], bps: &[f64]) -> Result<PathPlan, Infallible> {
    debug_assert_eq!(probed.len(), bps.len(), "probed/bps arity");
    let mut excluded = Vec::new();
    let mut live_bps = Vec::new();
    for (i, rate) in bps.iter().enumerate() {
        if *rate <= 0.0 {
            excluded.push(i);
        } else {
            live_bps.push(*rate);
        }
    }
    let weights = match live_bps.iter().copied().fold(f64::INFINITY, f64::min) {
        min if min.is_finite() && min > 0.0 => live_bps.iter().map(|b| b / min).collect(),
        _ => vec![1.0], // all dead: degrade to the single default path
    };
    Ok(PathPlan {
        weights,
        excluded,
        probed: probed.to_vec(),
    })
}

/// Split `pieces` across the plan's live paths — weighted round-robin by
/// largest remainder: each path floors its exact share, leftover pieces
/// go to the largest fractional shares (ties to the lower index, which is
/// the faster path). Deterministic by construction.
pub fn assign(plan: &PathPlan, pieces: u64) -> Vec<u64> {
    let n = plan.weights.len();
    if n == 0 {
        // Degrade: everything to the default path, reported as one count.
        return vec![pieces];
    }
    if n == 1 {
        return vec![pieces];
    }
    let total: f64 = plan.weights.iter().sum();
    let mut exact: Vec<(u64, f64)> = plan
        .weights
        .iter()
        .map(|w| {
            let share = *w / total * pieces as f64;
            (share.floor() as u64, share.fract())
        })
        .collect();
    let mut assigned: u64 = exact.iter().map(|(q, _)| *q).sum();
    // Largest-remainder pass.
    while assigned < pieces {
        let pick = (0..n)
            .max_by(|a, b| {
                exact[*a]
                    .1
                    .partial_cmp(&exact[*b].1)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(0);
        exact[pick].0 += 1;
        exact[pick].1 = 0.0; // spent remainder cannot win twice
        assigned += 1;
    }
    exact.into_iter().map(|(q, _)| q).collect()
}

/// The per-path AIMD signal (the outer loop over #43's per-session
/// realign rules).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// The path carried a piece clean: additive increase.
    Clean,
    /// The path errored: multiplicative decrease.
    ErrorSpike,
}

/// The additive-increase step.
pub const WEIGHT_STEP: f64 = 0.25;
/// The multiplicative-decrease factor.
pub const WEIGHT_DROP: f64 = 0.5;
/// The weight floor — a starving path slows but never zero-weight
/// vanishes (zeroing is the PROBER's exclusion call, journaled).
pub const WEIGHT_FLOOR: f64 = 1.0;
/// The weight ceiling — borrowing slack is bounded so a fast path cannot
/// starve the fleet's honest slow path.
pub const WEIGHT_CEILING: f64 = 4.0;

/// One AIMD step for one path's weight.
pub fn step_weight(w: f64, signal: Signal) -> f64 {
    match signal {
        Signal::Clean => (w + WEIGHT_STEP).min(WEIGHT_CEILING),
        Signal::ErrorSpike => (w * WEIGHT_DROP).max(WEIGHT_FLOOR),
    }
}

/// Compose the tus lane's partial-session count across paths: N partials
/// per path (the #50 declared ceiling) × P live paths.
pub fn compose_tus_partials(n_per_path: u32, p_paths: u32) -> u32 {
    n_per_path.saturating_mul(p_paths)
}

/// The composed count, clamped by the lane's declared global ceiling —
/// composition multiplies within what the plan allows.
pub fn compose_tus_partials_clamped(n_per_path: u32, p_paths: u32, ceiling: u32) -> u32 {
    compose_tus_partials(n_per_path, p_paths).min(ceiling)
}

/// The `path_plan` journal payload (probe boundary, issue design §5).
pub fn journal_payload(plan: &PathPlan, bps: &[f64]) -> serde_json::Value {
    let live = plan.live_indexes();
    let paths: Vec<serde_json::Value> = plan
        .probed
        .iter()
        .enumerate()
        .map(|(i, p)| {
            // weights are compacted over the live set; recover this
            // probed index's slot inside it.
            let live_pos = live.iter().position(|&l| l == i);
            serde_json::json!({
                "path": p.name(),
                "mode": match p {
                    NetPath::Default => "default",
                    NetPath::SourceAddr(_) => "source_addr",
                    NetPath::BindDevice(_) => "bind_device",
                },
                "bps": bps.get(i).copied().unwrap_or(0.0),
                "excluded": plan.excluded.contains(&i),
                "weight": live_pos.map(|pos| plan.weights[pos]),
            })
        })
        .collect();
    serde_json::json!({ "paths": paths })
}
