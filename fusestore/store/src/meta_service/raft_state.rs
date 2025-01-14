// Copyright 2020-2021 The Datafuse Authors.
//
// SPDX-License-Identifier: Apache-2.0.

use async_raft::storage::HardState;
use common_exception::ErrorCode;
use common_exception::ToErrorCode;
use common_tracing::tracing;

use crate::configs;
use crate::meta_service::sled_serde::SledOrderedSerde;
use crate::meta_service::NodeId;
use crate::meta_service::SledSerde;

/// Raft state stores everything else other than log and state machine, which includes:
/// id: NodeId,
/// hard_state:
///      current_term,
///      voted_for,
///
#[derive(Debug, Clone)]
pub struct RaftState {
    pub id: NodeId,

    /// A unique prefix for opening multiple RaftState in a same sled::Db
    // tree_prefix: String,
    tree: sled::Tree,
}

const K_RAFT_STATE: &str = "raft_state";
const K_ID: &str = "id";
const K_HARD_STATE: &str = "hard_state";

impl SledSerde for HardState {}

impl RaftState {
    /// Open/create a raft state in a sled db.
    /// 1. If `open` is `Some`,  it tries to open an existent RaftState if there is one.
    /// 2. If `create` is `Some`, it tries to initialize a new RaftState if there is not one.
    /// If none of them is `Some`, it is a programming error and will panic.
    #[tracing::instrument(level = "info", skip(db))]
    pub async fn open_create(
        db: &sled::Db,
        open: Option<()>,
        create: Option<&configs::Config>,
    ) -> common_exception::Result<(RaftState, bool)> {
        let t = db
            .open_tree(K_RAFT_STATE)
            .map_err_to_code(ErrorCode::MetaStoreDamaged, || {
                format!("open tree raft_state: name={}", "")
            })?;

        let curr_id = t
            .get(K_ID)
            .map_err_to_code(ErrorCode::MetaStoreDamaged, || "get id")?;

        let curr_id = match curr_id {
            Some(id) => Some(NodeId::de(id)?),
            None => None,
        };

        let (rs, is_open) = match (curr_id, open, create) {
            (Some(curr_id), Some(_), Some(_)) => Self::open(t, curr_id)?,
            (Some(curr_id), Some(_), None) => Self::open(t, curr_id)?,
            (Some(x), None, Some(_)) => {
                return Err(ErrorCode::MetaStoreAlreadyExists(format!(
                    "raft state present id={}, can not create",
                    x
                )));
            }
            (Some(_), None, None) => panic!("no open no create"),
            (None, Some(_), Some(&ref config)) => Self::create(t, config.id).await?,
            (None, Some(_), None) => {
                return Err(ErrorCode::MetaStoreNotFound(
                    "raft state absent, can not open",
                ));
            }
            (None, None, Some(&ref config)) => Self::create(t, config.id).await?,
            (None, None, None) => panic!("no open no create"),
        };

        Ok((rs, is_open))
    }

    fn open(t: sled::Tree, curr_id: NodeId) -> common_exception::Result<(RaftState, bool)> {
        Ok((
            RaftState {
                id: curr_id,
                tree: t,
            },
            true,
        ))
    }

    async fn create(tree: sled::Tree, id: NodeId) -> common_exception::Result<(RaftState, bool)> {
        let id_ivec = id.ser()?;

        tree.insert(K_ID, id_ivec)
            .map_err_to_code(ErrorCode::MetaStoreDamaged, || "write id")?;

        tracing::info!("inserted id: {}", K_ID);

        tree.flush_async()
            .await
            .map_err_to_code(ErrorCode::MetaStoreDamaged, || "flush raft state creation")?;

        tracing::info!("flushed");

        Ok((RaftState { id, tree }, false))
    }

    pub async fn write_hard_state(&self, hs: &HardState) -> common_exception::Result<()> {
        self.tree
            .insert(K_HARD_STATE, hs.ser()?)
            .map_err_to_code(ErrorCode::MetaStoreDamaged, || "write hard_state")?;

        self.tree
            .flush_async()
            .await
            .map_err_to_code(ErrorCode::MetaStoreDamaged, || "flush hard_state")?;
        Ok(())
    }

    pub async fn read_hard_state(&self) -> common_exception::Result<Option<HardState>> {
        let hs = self
            .tree
            .get(K_HARD_STATE)
            .map_err_to_code(ErrorCode::MetaStoreDamaged, || "read hard_state")?;

        let hs = match hs {
            Some(hs) => hs,
            None => {
                return Ok(None);
            }
        };

        let hs = HardState::de(hs)?;
        Ok(Some(hs))
    }
}
