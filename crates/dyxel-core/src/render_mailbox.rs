// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use dyxel_render_api::RenderPackage;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// Single-slot latest-wins mailbox for RenderPackage snapshots.
///
/// Logic worker commits new packages here; Render worker reads stable snapshots.
/// Higher epochs always supersede lower ones.  There is no queue — only the
/// latest committed package is visible.
pub struct RenderMailbox {
    latest_epoch: AtomicU64,
    latest_package: RwLock<Arc<RenderPackage>>,
}

impl RenderMailbox {
    pub fn new() -> Self {
        Self {
            latest_epoch: AtomicU64::new(0),
            latest_package: RwLock::new(Arc::new(RenderPackage::new(
                (0, 0),
                None,
                Vec::new(),
            ))),
        }
    }

    /// Commit a new package, replacing any previous one (latest-wins).
    pub fn commit(&self, epoch: u64, package: Arc<RenderPackage>) {
        *self.latest_package.write().unwrap() = package;
        self.latest_epoch.store(epoch, Ordering::Release);
    }

    /// Read a stable snapshot of the latest committed package.
    ///
    /// Returns `(epoch, package)`.  The epoch may be 0 if nothing has been
    /// committed yet.
    pub fn snapshot(&self) -> (u64, Arc<RenderPackage>) {
        let epoch = self.latest_epoch.load(Ordering::Acquire);
        let pkg = self.latest_package.read().unwrap().clone();
        (epoch, pkg)
    }
}

impl Default for RenderMailbox {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailbox_commit_replaces_previous_epoch() {
        let mailbox = RenderMailbox::new();
        let p1 = Arc::new(RenderPackage::new((100, 100), None, Vec::new()));
        let p2 = Arc::new(RenderPackage::new((200, 200), None, Vec::new()));

        mailbox.commit(1, p1);
        mailbox.commit(2, p2.clone());

        let (epoch, snapshot) = mailbox.snapshot();
        assert_eq!(epoch, 2);
        assert_eq!(snapshot.viewport, (200, 200));
    }

    #[test]
    fn mailbox_snapshot_is_stable() {
        let mailbox = RenderMailbox::new();
        let p1 = Arc::new(RenderPackage::new((100, 100), None, Vec::new()));

        mailbox.commit(1, p1.clone());

        let (_, snap1) = mailbox.snapshot();
        // Overwrite with a newer package
        let p2 = Arc::new(RenderPackage::new((300, 300), None, Vec::new()));
        mailbox.commit(2, p2);

        // The old snapshot must still reflect the package we read at that time
        assert_eq!(snap1.viewport, (100, 100));
    }

    #[test]
    fn mailbox_uncommitted_returns_zero_epoch() {
        let mailbox = RenderMailbox::new();
        let (epoch, snapshot) = mailbox.snapshot();
        assert_eq!(epoch, 0);
        assert_eq!(snapshot.viewport, (0, 0));
    }
}
