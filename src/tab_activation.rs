use crate::app::TabId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TabActivationId(pub(crate) u64);

impl TabActivationId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabActivationIntent {
    id: TabActivationId,
    from: TabId,
    to: TabId,
}

impl TabActivationIntent {
    pub(crate) const fn new(id: TabActivationId, from: TabId, to: TabId) -> Self {
        Self { id, from, to }
    }

    pub const fn id(self) -> TabActivationId {
        self.id
    }

    pub const fn from(self) -> TabId {
        self.from
    }

    pub const fn to(self) -> TabId {
        self.to
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabActivationStart {
    intent: TabActivationIntent,
    superseded: Option<TabActivationIntent>,
}

impl TabActivationStart {
    pub(crate) const fn new(
        intent: TabActivationIntent,
        superseded: Option<TabActivationIntent>,
    ) -> Self {
        Self { intent, superseded }
    }

    pub const fn intent(self) -> TabActivationIntent {
        self.intent
    }

    pub const fn superseded(self) -> Option<TabActivationIntent> {
        self.superseded
    }
}
