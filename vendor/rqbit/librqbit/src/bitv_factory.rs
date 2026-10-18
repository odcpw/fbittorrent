use rqbit_runtime::Cx;

use crate::{api::TorrentIdOrHash, bitv::BitV, type_aliases::BF};

#[async_trait::async_trait]
pub trait BitVFactory: Send + Sync {
    async fn load(&self, cx: &Cx, id: TorrentIdOrHash) -> anyhow::Result<Option<Box<dyn BitV>>>;
    async fn clear(&self, cx: &Cx, id: TorrentIdOrHash) -> anyhow::Result<()>;
    async fn store_initial_check(
        &self,
        cx: &Cx,
        id: TorrentIdOrHash,
        b: BF,
    ) -> anyhow::Result<Box<dyn BitV>>;
}

pub struct NonPersistentBitVFactory {}

#[async_trait::async_trait]
impl BitVFactory for NonPersistentBitVFactory {
    async fn load(&self, cx: &Cx, _: TorrentIdOrHash) -> anyhow::Result<Option<Box<dyn BitV>>> {
        cx.checkpoint()?;
        Ok(None)
    }

    async fn clear(&self, cx: &Cx, _id: TorrentIdOrHash) -> anyhow::Result<()> {
        cx.checkpoint()?;
        Ok(())
    }

    async fn store_initial_check(
        &self,
        cx: &Cx,
        _id: TorrentIdOrHash,
        b: BF,
    ) -> anyhow::Result<Box<dyn BitV>> {
        cx.checkpoint()?;
        Ok(Box::new(b))
    }
}
