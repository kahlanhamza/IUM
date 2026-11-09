//! Own-drift convergence drill (issue #43) — the timed-out PATCH's OWN
//! bytes landing on the server are drift to ABSORB, not a client bug to
//! fail loudly on.
//!
//! The field shape (owner's 3 KB/s tunnel): a PATCH dies transport-side
//! (idle cut / tunnel reset) while the server has already persisted part of
//! its body — tusd keeps what arrived before the socket died (#33/#37). The
//! adapter's realign dance exists for exactly this; but when the REALIGN
//! PATCH dies the same way, the current code maps the second conflict to
//! `Unknown("409 again after realign — server moved under the lease")` →
//! `Action::Fail` → `Failed` is TERMINAL. On a slow link that is not a
//! client bug — it is the same own-drift, one round later.
//!
//! This drill: kill TWO PATCHes mid-body (server persists what it read
//! each time), let the third complete. The call must CONVERGE to a full
//! confirm in ONE `upload_chunk` invocation — bounded realign rounds,
//! honest waste reckoning, journal converges to server truth.

#[allow(dead_code)] // helpers are shared with tests/real_data.rs etc.
mod common;

use std::time::Duration;

use common::tus_mock::{PatchBehavior, TusMock};
use ium_core::adapter::{ProgressReporter, TransferAdapter};
use ium_core::adapter_tus::TusAdapter;
use ium_core::{JobSpec, Lease};

const KIB: u64 = 1024;
const SPAN: u64 = 256 * KIB;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_midbody_deaths_converge_in_one_call() {
    let mock = TusMock::spawn(
        vec![
            // 1st PATCH: server persists 64 KiB, then the "tunnel" dies.
            PatchBehavior::KillAfterReading { bytes: 64 * KIB },
            // 2nd PATCH (the realign): the SAME death, 32 KiB further.
            PatchBehavior::KillAfterReading { bytes: 32 * KIB },
            // 3rd PATCH: the link recovered — everything lands.
            PatchBehavior::Accept,
        ],
        SPAN,
    );

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("source.bin");
    std::fs::write(
        &path,
        (0..SPAN as usize)
            .map(|i| (i % 251) as u8)
            .collect::<Vec<u8>>(),
    )
    .expect("write source");

    let spec = JobSpec {
        source_path: path.to_string_lossy().into_owned(),
        source_size: SPAN,
        adapter: "tus".into(),
        endpoint: mock.endpoint(),
    };
    let adapter = TusAdapter::new();
    let probe = adapter
        .probe(&spec)
        .await
        .expect("creation must succeed on a healthy request");
    assert!(probe.resumable);

    let lease = Lease {
        idx: 0,
        begin: 0,
        end: SPAN,
        resume_from: 0,
        attempt: 0,
    };
    let progress = ProgressReporter::new();

    // One call must ride out BOTH deaths and land the whole span.
    let confirm = tokio::time::timeout(
        Duration::from_secs(30),
        adapter.upload_chunk(&spec, &lease, &progress),
    )
    .await
    .expect("upload_chunk must converge within the drill budget (two realign rounds)")
    .expect("own-drift must be absorbed, never mapped to Unknown->Fail");

    assert_eq!(
        confirm.bytes_confirmed, SPAN,
        "the server holds the whole span — every killed attempt's landed bytes are confirmed"
    );
    assert_eq!(mock.offset(), SPAN, "server offset must reach the span end");
    assert_eq!(
        mock.patches_killed(),
        2,
        "the drill's two kill-shots must both have fired"
    );
    // The #37 reckoning, as the exact identity it defines: every counted
    // wire byte is either server-persisted or measured waste —
    // `moved() − wasted() == bytes the server actually took`. The old
    // `wasted() > 0` asserted one side of a documented client↔server
    // scheduling race (the waste size is "0–48 KiB quiet, 192 KiB on a
    // loaded runner" — 0 is IN range) and flaked the gate on a quiet box
    // (issue #95 round-2 gate). The identity is deterministic on both.
    assert_eq!(
        progress.moved() - progress.wasted(),
        SPAN,
        "the accounting identity: persisted bytes + measured waste == counted wire bytes"
    );
    // Byte fidelity: whatever route the bytes took (kill + realign + accept),
    // the stored content is exactly the source's prefix.
    assert_eq!(
        mock.stored(),
        std::fs::read(&path).expect("read source")[..SPAN as usize],
        "server store must be byte-exact after convergence"
    );
}
