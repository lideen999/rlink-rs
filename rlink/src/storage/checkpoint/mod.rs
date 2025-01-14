use crate::core::backend::CheckpointBackend;
use crate::core::checkpoint::Checkpoint;
use crate::core::runtime::CheckpointId;
use crate::storage::checkpoint::memory_checkpoint_storage::MemoryCheckpointStorage;
use crate::storage::checkpoint::mysql_checkpoint_storage::MySqlCheckpointStorage;

pub mod memory_checkpoint_storage;
pub mod mysql_checkpoint_storage;

pub trait TCheckpointStorage {
    fn save(
        &mut self,
        application_name: &str,
        application_id: &str,
        checkpoint_id: CheckpointId,
        finish_cks: Vec<Checkpoint>,
        ttl: u64,
    ) -> anyhow::Result<()>;

    fn load(
        &mut self,
        application_name: &str,
        application_id: &str,
    ) -> anyhow::Result<Vec<Checkpoint>>;

    fn load_by_checkpoint_id(
        &mut self,
        application_name: &str,
        application_id: &str,
        checkpoint_id: CheckpointId,
    ) -> anyhow::Result<Vec<Checkpoint>>;
}

pub enum CheckpointStorage {
    MemoryCheckpointStorage(MemoryCheckpointStorage),
    MySqlCheckpointStorage(MySqlCheckpointStorage),
}

impl CheckpointStorage {
    pub fn new(checkpoint_backend: &CheckpointBackend) -> Self {
        match checkpoint_backend {
            CheckpointBackend::Memory => {
                CheckpointStorage::MemoryCheckpointStorage(MemoryCheckpointStorage::new())
            }
            CheckpointBackend::MySql { endpoint, table } => {
                CheckpointStorage::MySqlCheckpointStorage(MySqlCheckpointStorage::new(
                    endpoint.clone(),
                    table.clone(),
                ))
            }
        }
    }
}

impl TCheckpointStorage for CheckpointStorage {
    fn save(
        &mut self,
        application_name: &str,
        application_id: &str,
        checkpoint_id: CheckpointId,
        finish_cks: Vec<Checkpoint>,
        ttl: u64,
    ) -> anyhow::Result<()> {
        match self {
            CheckpointStorage::MemoryCheckpointStorage(storage) => storage.save(
                application_name,
                application_id,
                checkpoint_id,
                finish_cks,
                ttl,
            ),
            CheckpointStorage::MySqlCheckpointStorage(storage) => storage.save(
                application_name,
                application_id,
                checkpoint_id,
                finish_cks,
                ttl,
            ),
        }
    }

    fn load(
        &mut self,
        application_name: &str,
        application_id: &str,
    ) -> anyhow::Result<Vec<Checkpoint>> {
        match self {
            CheckpointStorage::MemoryCheckpointStorage(storage) => {
                storage.load(application_name, application_id)
            }
            CheckpointStorage::MySqlCheckpointStorage(storage) => {
                storage.load(application_name, application_id)
            }
        }
    }

    fn load_by_checkpoint_id(
        &mut self,
        application_name: &str,
        application_id: &str,
        checkpoint_id: CheckpointId,
    ) -> anyhow::Result<Vec<Checkpoint>> {
        match self {
            CheckpointStorage::MemoryCheckpointStorage(storage) => {
                storage.load_by_checkpoint_id(application_name, application_id, checkpoint_id)
            }
            CheckpointStorage::MySqlCheckpointStorage(storage) => {
                storage.load_by_checkpoint_id(application_name, application_id, checkpoint_id)
            }
        }
    }
}
