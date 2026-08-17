//! BUILD_PLAN M3 item 11: the denylist veto against hardlinked files, over
//! real temp-dir fixtures (never real user directories — see CLAUDE.md
//! golden rules). A hardlink is two independent directory entries pointing
//! at the same underlying data; the concern this covers is that sharing
//! data with a protected-path entry must never let a move affect the
//! protected entry, and must never let the protection check itself be
//! confused by that shared identity into either direction: never protect
//! less than it should, and (just as important for user trust) never
//! refuse a genuinely unprotected item just because some *other* hardlink
//! of the same data happens to sit somewhere protected.

use std::fs;
use std::path::PathBuf;

use reclaim_action::{quarantine, Error, QuarantineRequest};
use reclaim_core::{ProtectedPaths, ReclaimMethod, Tier};
use reclaim_store::Store;

fn request(source_path: PathBuf) -> QuarantineRequest {
    QuarantineRequest {
        source_path,
        size_bytes: 0,
        size_on_disk: 0,
        tier: Tier::Safe,
        reclaim_method: ReclaimMethod::Move,
    }
}

/// Quarantining a Safe-tier hardlink must never touch its sibling link at a
/// protected path -- moving one directory entry of a multiply-linked file
/// is a metadata operation on that entry alone; the shared data and every
/// other entry pointing at it must be completely unaffected.
#[test]
fn quarantining_a_hardlink_leaves_its_protected_sibling_link_untouched() {
    let base = tempfile::tempdir().unwrap();
    let protected_root = base.path().join("Documents");
    let safe_root = base.path().join("project");
    fs::create_dir_all(&protected_root).unwrap();
    fs::create_dir_all(&safe_root).unwrap();

    // One file, two links: one under the protected root, one under a
    // Safe-tier candidate directory.
    let protected_link = protected_root.join("shared.txt");
    fs::write(&protected_link, b"irreplaceable data").unwrap();
    let node_modules = safe_root.join("node_modules");
    fs::create_dir_all(&node_modules).unwrap();
    let safe_link = node_modules.join("shared.txt");
    fs::hard_link(&protected_link, &safe_link).unwrap();

    let store = Store::open_in_memory().unwrap();
    let protected = ProtectedPaths::from_roots([protected_root.to_string_lossy().into_owned()]);
    let staging_root = base.path().join("staging");

    let dest = quarantine(
        &store,
        &protected,
        &staging_root,
        30,
        request(node_modules.clone()),
    )
    .expect("the Safe-tier link must quarantine normally");

    assert!(
        !node_modules.exists(),
        "the Safe-tier directory entry must be gone"
    );
    assert!(
        dest.join("shared.txt").exists(),
        "it must exist at the quarantine destination"
    );

    // The protected sibling link: untouched, in place, same content.
    assert!(
        protected_link.exists(),
        "the protected sibling link must still exist -- it shares data, not a directory entry"
    );
    assert_eq!(
        fs::read(&protected_link).unwrap(),
        b"irreplaceable data",
        "the protected sibling link's content must be byte-identical to before"
    );
}

/// The inverse direction: a hardlink whose *own* path is under a protected
/// root must be refused, even though a sibling link to the same data sits
/// at an unprotected, otherwise-Safe-looking path. Shared file identity
/// must never be a bypass for the literal-path denylist check.
#[test]
fn a_hardlink_at_a_protected_path_is_refused_even_with_an_unprotected_sibling() {
    let base = tempfile::tempdir().unwrap();
    let protected_root = base.path().join("Documents");
    let safe_root = base.path().join("project");
    fs::create_dir_all(&protected_root).unwrap();
    fs::create_dir_all(&safe_root).unwrap();

    let protected_link = protected_root.join("shared.txt");
    fs::write(&protected_link, b"irreplaceable data").unwrap();
    let node_modules = safe_root.join("node_modules");
    fs::create_dir_all(&node_modules).unwrap();
    let safe_link = node_modules.join("shared.txt");
    fs::hard_link(&protected_link, &safe_link).unwrap();

    let store = Store::open_in_memory().unwrap();
    let protected = ProtectedPaths::from_roots([protected_root.to_string_lossy().into_owned()]);
    let staging_root = base.path().join("staging");

    let err = quarantine(
        &store,
        &protected,
        &staging_root,
        30,
        request(protected_link.clone()),
    )
    .expect_err("a request naming the protected path directly must be refused");

    assert!(matches!(err, Error::ProtectedPath(_)));
    assert!(
        protected_link.exists(),
        "protected link must remain in place"
    );
    assert!(
        safe_link.exists(),
        "its unprotected sibling must be untouched too -- nothing moved at all"
    );
    assert_eq!(fs::read(&protected_link).unwrap(), b"irreplaceable data");
}
