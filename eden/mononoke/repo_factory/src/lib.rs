/*
 * Copyright (c) Facebook, Inc. and its affiliates.
 *
 * This software may be used and distributed according to the terms of the
 * GNU General Public License version 2.
 */

//! Repository factory.
#![feature(trait_alias)]

use context::CoreContext;
use skiplist::{ArcSkiplistIndex, SkiplistIndex};
use sql_construct::{SqlConstruct, SqlConstructFromDatabaseConfig};
use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::num::NonZeroUsize;
use std::sync::Arc;
use tunables::tunables;

use anyhow::{Context, Result};
use async_once_cell::AsyncOnceCell;
use blobstore::Blobstore;
use blobstore_factory::{
    default_scrub_handler, make_blobstore, make_metadata_sql_factory, ComponentSamplingHandler,
    MetadataSqlFactory, ScrubHandler,
};
use bonsai_git_mapping::{ArcBonsaiGitMapping, SqlBonsaiGitMappingConnection};
use bonsai_globalrev_mapping::{
    ArcBonsaiGlobalrevMapping, CachingBonsaiGlobalrevMapping, SqlBonsaiGlobalrevMapping,
};
use bonsai_hg_mapping::{ArcBonsaiHgMapping, CachingBonsaiHgMapping, SqlBonsaiHgMappingBuilder};
use bonsai_svnrev_mapping::{
    ArcRepoBonsaiSvnrevMapping, BonsaiSvnrevMapping, CachingBonsaiSvnrevMapping,
    RepoBonsaiSvnrevMapping, SqlBonsaiSvnrevMapping,
};
use bookmarks::{ArcBookmarkUpdateLog, ArcBookmarks, CachedBookmarks};
use cacheblob::{
    new_cachelib_blobstore_no_lease, new_memcache_blobstore, CachelibBlobstoreOptions,
    InProcessLease, LeaseOps, MemcacheOps,
};
use changeset_fetcher::{ArcChangesetFetcher, SimpleChangesetFetcher};
use changesets::ArcChangesets;
use changesets_impl::{CachingChangesets, SqlChangesetsBuilder};
use context::SessionContainer;
use dbbookmarks::{ArcSqlBookmarks, SqlBookmarksBuilder};
#[cfg(fbcode_build)]
use derived_data_client_library::Client as DerivationServiceClient;
use derived_data_manager::{ArcDerivedDataManagerSet, DerivedDataManagerSet};
use derived_data_remote::{DerivationClient, RemoteDerivationOptions};
use environment::{Caching, MononokeEnvironment};
use ephemeral_blobstore::{
    ArcRepoEphemeralBlobstore, RepoEphemeralBlobstore, RepoEphemeralBlobstoreBuilder,
};
use fbinit::FacebookInit;
use filenodes::ArcFilenodes;
use filestore::{ArcFilestoreConfig, FilestoreConfig};
use futures_watchdog::WatchdogExt;
use mercurial_mutation::{ArcHgMutationStore, SqlHgMutationStoreBuilder};
use metaconfig_types::{
    ArcRepoConfig, BlobConfig, CensoredScubaParams, CommonConfig, MetadataDatabaseConfig,
    Redaction, RedactionConfig, RepoConfig,
};
use mutable_renames::{ArcMutableRenames, MutableRenames, SqlMutableRenamesStore};
use newfilenodes::NewFilenodesBuilder;
use parking_lot::Mutex;
use phases::{ArcSqlPhasesFactory, SqlPhasesFactory};
use pushrebase_mutation_mapping::{
    ArcPushrebaseMutationMapping, SqlPushrebaseMutationMappingConnection,
};
use readonlyblob::ReadOnlyBlobstore;
use redactedblobstore::{ArcRedactionConfigBlobstore, RedactionConfigBlobstore};
use redactedblobstore::{RedactedBlobs, SqlRedactedContentStore};
use repo_blobstore::{ArcRepoBlobstore, RepoBlobstore};
use repo_derived_data::{ArcRepoDerivedData, RepoDerivedData};
use repo_identity::{ArcRepoIdentity, RepoIdentity};
use requests_table::{ArcLongRunningRequestsQueue, SqlLongRunningRequestsQueue};
use scuba_ext::MononokeScubaSampleBuilder;
use segmented_changelog::{new_server_segmented_changelog, SegmentedChangelogSqlConnections};
use segmented_changelog_types::ArcSegmentedChangelog;
use slog::o;
use sql::SqlConnectionsWithSchema;
use thiserror::Error;
use virtually_sharded_blobstore::VirtuallyShardedBlobstore;

pub use blobstore_factory::{BlobstoreOptions, ReadOnlyStorage};

const DERIVED_DATA_LEASE: &str = "derived-data-lease";

#[derive(Clone)]
struct RepoFactoryCache<K: Clone + Eq + Hash, V: Clone> {
    cache: Arc<Mutex<HashMap<K, Arc<AsyncOnceCell<V>>>>>,
}

impl<K: Clone + Eq + Hash, V: Clone> RepoFactoryCache<K, V> {
    fn new() -> Self {
        RepoFactoryCache {
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn get_or_try_init<F, Fut>(&self, key: &K, init: F) -> Result<V>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<V>>,
    {
        let cell = {
            let mut cache = self.cache.lock();
            match cache.get(key) {
                Some(cell) => {
                    if let Some(value) = cell.get() {
                        return Ok(value.clone());
                    }
                    cell.clone()
                }
                None => {
                    let cell = Arc::new(AsyncOnceCell::new());
                    cache.insert(key.clone(), cell.clone());
                    cell
                }
            }
        };
        let value = cell.get_or_try_init(init).await?;
        Ok(value.clone())
    }
}

pub trait RepoFactoryOverride<T> = Fn(T) -> T + Send + Sync + 'static;

#[derive(Clone)]
pub struct RepoFactory {
    pub env: Arc<MononokeEnvironment>,
    censored_scuba_params: CensoredScubaParams,
    redaction_config: RedactionConfig,
    sql_factories: RepoFactoryCache<MetadataDatabaseConfig, Arc<MetadataSqlFactory>>,
    sql_connections: RepoFactoryCache<MetadataDatabaseConfig, SqlConnectionsWithSchema>,
    blobstores: RepoFactoryCache<BlobConfig, Arc<dyn Blobstore>>,
    redacted_blobs: RepoFactoryCache<MetadataDatabaseConfig, Arc<RedactedBlobs>>,
    blobstore_override: Option<Arc<dyn RepoFactoryOverride<Arc<dyn Blobstore>>>>,
    scrub_handler: Arc<dyn ScrubHandler>,
    blobstore_component_sampler: Option<Arc<dyn ComponentSamplingHandler>>,
}

impl RepoFactory {
    pub fn new(env: Arc<MononokeEnvironment>, common: &CommonConfig) -> RepoFactory {
        RepoFactory {
            env,
            censored_scuba_params: common.censored_scuba_params.clone(),
            sql_factories: RepoFactoryCache::new(),
            sql_connections: RepoFactoryCache::new(),
            blobstores: RepoFactoryCache::new(),
            redacted_blobs: RepoFactoryCache::new(),
            blobstore_override: None,
            scrub_handler: default_scrub_handler(),
            blobstore_component_sampler: None,
            redaction_config: common.redaction_config.clone(),
        }
    }

    pub fn with_blobstore_override(
        &mut self,
        blobstore_override: impl RepoFactoryOverride<Arc<dyn Blobstore>>,
    ) -> &mut Self {
        self.blobstore_override = Some(Arc::new(blobstore_override));
        self
    }

    pub fn with_scrub_handler(&mut self, scrub_handler: Arc<dyn ScrubHandler>) -> &mut Self {
        self.scrub_handler = scrub_handler;
        self
    }

    pub fn with_blobstore_component_sampler(
        &mut self,
        handler: Arc<dyn ComponentSamplingHandler>,
    ) -> &mut Self {
        self.blobstore_component_sampler = Some(handler);
        self
    }

    pub async fn sql_factory(
        &self,
        config: &MetadataDatabaseConfig,
    ) -> Result<Arc<MetadataSqlFactory>> {
        self.sql_factories
            .get_or_try_init(config, || async move {
                let sql_factory = make_metadata_sql_factory(
                    self.env.fb,
                    config.clone(),
                    self.env.mysql_options.clone(),
                    self.env.readonly_storage,
                )
                .watched(&self.env.logger)
                .await?;
                Ok(Arc::new(sql_factory))
            })
            .await
    }

    async fn sql_connections(
        &self,
        config: &MetadataDatabaseConfig,
    ) -> Result<SqlConnectionsWithSchema> {
        let sql_factory = self.sql_factory(config).await?;
        self.sql_connections
            .get_or_try_init(config, || async move {
                sql_factory
                    .make_primary_connections("metadata".to_string())
                    .await
            })
            .await
    }

    async fn open<T: SqlConstruct>(&self, config: &MetadataDatabaseConfig) -> Result<T> {
        let sql_connections = match config {
            // For sqlite cache the connections to save reopening the file
            MetadataDatabaseConfig::Local(_) => self.sql_connections(config).await?,
            // TODO(ahornby) for other dbs the label can be part of connection identity in stats so don't reuse
            _ => {
                self.sql_factory(config)
                    .await?
                    .make_primary_connections(T::LABEL.to_string())
                    .await?
            }
        };
        T::from_connections_with_schema(sql_connections)
    }

    async fn blobstore_no_cache(&self, config: &BlobConfig) -> Result<Arc<dyn Blobstore>> {
        make_blobstore(
            self.env.fb,
            config.clone(),
            &self.env.mysql_options,
            self.env.readonly_storage,
            &self.env.blobstore_options,
            &self.env.logger,
            &self.env.config_store,
            &self.scrub_handler,
            self.blobstore_component_sampler.as_ref(),
        )
        .watched(&self.env.logger)
        .await
    }

    async fn repo_blobstore_from_blobstore(
        &self,
        repo_identity: &ArcRepoIdentity,
        repo_config: &ArcRepoConfig,
        blobstore: &Arc<dyn Blobstore>,
    ) -> Result<RepoBlobstore> {
        let mut blobstore = blobstore.clone();
        if self.env.readonly_storage.0 {
            blobstore = Arc::new(ReadOnlyBlobstore::new(blobstore));
        }

        let redacted_blobs = match repo_config.redaction {
            Redaction::Enabled => {
                let redacted_blobs = self
                    .redacted_blobs(self.ctx(None), &repo_config.storage_config.metadata)
                    .await?;
                Some(redacted_blobs)
            }
            Redaction::Disabled => None,
        };

        let censored_scuba_builder = self.censored_scuba_builder()?;

        let repo_blobstore = RepoBlobstore::new(
            blobstore,
            redacted_blobs,
            repo_identity.id(),
            censored_scuba_builder,
        );

        Ok(repo_blobstore)
    }

    async fn blobstore(&self, config: &BlobConfig) -> Result<Arc<dyn Blobstore>> {
        self.blobstores
            .get_or_try_init(config, || async move {
                let mut blobstore = self.blobstore_no_cache(config).await?;

                match self.env.caching {
                    Caching::Enabled(cache_shards) => {
                        let fb = self.env.fb;
                        let memcache_blobstore = tokio::task::spawn_blocking(move || {
                            new_memcache_blobstore(fb, blobstore, "multiplexed", "")
                        })
                        .await??;
                        blobstore = cachelib_blobstore(
                            memcache_blobstore,
                            cache_shards,
                            &self.env.blobstore_options.cachelib_options,
                        )?
                    }
                    Caching::CachelibOnlyBlobstore(cache_shards) => {
                        blobstore = cachelib_blobstore(
                            blobstore,
                            cache_shards,
                            &self.env.blobstore_options.cachelib_options,
                        )?;
                    }
                    Caching::Disabled => {}
                };

                if let Some(blobstore_override) = &self.blobstore_override {
                    blobstore = blobstore_override(blobstore);
                }

                Ok(blobstore)
            })
            .await
    }

    async fn redacted_blobs(
        &self,
        ctx: CoreContext,
        db_config: &MetadataDatabaseConfig,
    ) -> Result<Arc<RedactedBlobs>> {
        self.redacted_blobs
            .get_or_try_init(db_config, || async move {
                let redacted_blobs = if tunables().get_redaction_config_from_xdb() {
                    let redacted_content_store =
                        self.open::<SqlRedactedContentStore>(db_config).await?;
                    // Fetch redacted blobs in a separate task so that slow polls
                    // in repo construction don't interfere with the SQL query.
                    tokio::task::spawn(async move {
                        redacted_content_store.get_all_redacted_blobs().await
                    })
                    .await??
                } else {
                    let blobstore = self.redaction_config_blobstore().await?;
                    RedactedBlobs::from_configerator(
                        &self.env.config_store,
                        &self.redaction_config.redaction_sets_location,
                        ctx,
                        blobstore,
                    )
                    .await?
                };
                Ok(Arc::new(redacted_blobs))
            })
            .await
    }

    pub async fn redaction_config_blobstore_from_config(
        &self,
        config: &BlobConfig,
    ) -> Result<ArcRedactionConfigBlobstore> {
        let blobstore = self.blobstore(&config).await?;
        Ok(Arc::new(RedactionConfigBlobstore::new(blobstore)))
    }

    fn ctx(&self, repo_identity: Option<&ArcRepoIdentity>) -> CoreContext {
        let logger = repo_identity
            .map(|id| {
                let repo_name = String::from(id.name());
                self.env.logger.new(o!("repo" => repo_name))
            })
            .unwrap_or_else(|| self.env.logger.new(o!()));
        let session = SessionContainer::new_with_defaults(self.env.fb);
        session.new_context(logger, self.env.scuba_sample_builder.clone())
    }

    /// Returns a named volatile pool if caching is enabled.
    fn maybe_volatile_pool(&self, name: &str) -> Result<Option<cachelib::VolatileLruCachePool>> {
        match self.env.caching {
            Caching::Enabled(_) => Ok(Some(volatile_pool(name)?)),
            _ => Ok(None),
        }
    }

    fn censored_scuba_builder(&self) -> Result<MononokeScubaSampleBuilder> {
        let mut builder = MononokeScubaSampleBuilder::with_opt_table(
            self.env.fb,
            self.censored_scuba_params.table.clone(),
        );
        builder.add_common_server_data();
        if let Some(scuba_log_file) = &self.censored_scuba_params.local_path {
            builder = builder.with_log_file(scuba_log_file)?;
        }
        Ok(builder)
    }
}

fn cache_pool(name: &str) -> Result<cachelib::LruCachePool> {
    Ok(cachelib::get_pool(name)
        .ok_or_else(|| RepoFactoryError::MissingCachePool(name.to_string()))?)
}

fn volatile_pool(name: &str) -> Result<cachelib::VolatileLruCachePool> {
    Ok(cachelib::get_volatile_pool(name)?
        .ok_or_else(|| RepoFactoryError::MissingCachePool(name.to_string()))?)
}

pub fn cachelib_blobstore<B: Blobstore + 'static>(
    blobstore: B,
    cache_shards: usize,
    options: &CachelibBlobstoreOptions,
) -> Result<Arc<dyn Blobstore>> {
    const BLOBSTORE_BLOBS_CACHE_POOL: &str = "blobstore-blobs";
    const BLOBSTORE_PRESENCE_CACHE_POOL: &str = "blobstore-presence";

    let blobstore: Arc<dyn Blobstore> = match NonZeroUsize::new(cache_shards) {
        Some(cache_shards) => {
            let blob_pool = volatile_pool(BLOBSTORE_BLOBS_CACHE_POOL)?;
            let presence_pool = volatile_pool(BLOBSTORE_PRESENCE_CACHE_POOL)?;

            Arc::new(VirtuallyShardedBlobstore::new(
                blobstore,
                blob_pool,
                presence_pool,
                cache_shards,
                options.clone(),
            ))
        }
        None => {
            let blob_pool = cache_pool(BLOBSTORE_BLOBS_CACHE_POOL)?;
            let presence_pool = cache_pool(BLOBSTORE_PRESENCE_CACHE_POOL)?;

            Arc::new(new_cachelib_blobstore_no_lease(
                blobstore,
                Arc::new(blob_pool),
                Arc::new(presence_pool),
                options.clone(),
            ))
        }
    };

    Ok(blobstore)
}

#[derive(Debug, Error)]
pub enum RepoFactoryError {
    #[error("Error opening changesets")]
    Changesets,

    #[error("Error opening bookmarks")]
    Bookmarks,

    #[error("Error opening phases")]
    Phases,

    #[error("Error opening bonsai-hg mapping")]
    BonsaiHgMapping,

    #[error("Error opening bonsai-git mapping")]
    BonsaiGitMapping,

    #[error("Error opening bonsai-globalrev mapping")]
    BonsaiGlobalrevMapping,

    #[error("Error opening bonsai-svnrev mapping")]
    BonsaiSvnrevMapping,

    #[error("Error opening pushrebase mutation mapping")]
    PushrebaseMutationMapping,

    #[error("Error opening filenodes")]
    Filenodes,

    #[error("Error opening hg mutation store")]
    HgMutationStore,

    #[error("Error opening segmented changelog")]
    SegmentedChangelog,

    #[error("Missing cache pool: {0}")]
    MissingCachePool(String),

    #[error("Error opening long-running request queue")]
    LongRunningRequestsQueue,

    #[error("Error opening mutable renames")]
    MutableRenames,
}

#[facet::factory(name: String, config: RepoConfig)]
impl RepoFactory {
    pub fn repo_config(&self, config: &RepoConfig) -> ArcRepoConfig {
        Arc::new(config.clone())
    }

    pub fn repo_identity(&self, name: &str, repo_config: &ArcRepoConfig) -> ArcRepoIdentity {
        Arc::new(RepoIdentity::new(repo_config.repoid, name.to_string()))
    }

    pub fn caching(&self) -> Caching {
        self.env.caching
    }

    pub async fn changesets(
        &self,
        repo_identity: &ArcRepoIdentity,
        repo_config: &ArcRepoConfig,
    ) -> Result<ArcChangesets> {
        let builder = self
            .open::<SqlChangesetsBuilder>(&repo_config.storage_config.metadata)
            .await
            .context(RepoFactoryError::Changesets)?;
        let changesets = builder.build(self.env.rendezvous_options, repo_identity.id());
        if let Some(pool) = self.maybe_volatile_pool("changesets")? {
            Ok(Arc::new(CachingChangesets::new(
                self.env.fb,
                Arc::new(changesets),
                pool,
            )))
        } else {
            Ok(Arc::new(changesets))
        }
    }

    pub fn changeset_fetcher(
        &self,
        repo_identity: &ArcRepoIdentity,
        changesets: &ArcChangesets,
    ) -> ArcChangesetFetcher {
        Arc::new(SimpleChangesetFetcher::new(
            changesets.clone(),
            repo_identity.id(),
        ))
    }

    pub async fn sql_bookmarks(
        &self,
        repo_config: &ArcRepoConfig,
        repo_identity: &ArcRepoIdentity,
    ) -> Result<ArcSqlBookmarks> {
        let sql_bookmarks = self
            .open::<SqlBookmarksBuilder>(&repo_config.storage_config.metadata)
            .await
            .context(RepoFactoryError::Bookmarks)?
            .with_repo_id(repo_identity.id());

        Ok(Arc::new(sql_bookmarks))
    }

    pub fn bookmarks(
        &self,
        sql_bookmarks: &ArcSqlBookmarks,
        repo_identity: &ArcRepoIdentity,
    ) -> ArcBookmarks {
        Arc::new(CachedBookmarks::new(
            sql_bookmarks.clone(),
            repo_identity.id(),
        ))
    }

    pub fn bookmark_update_log(&self, sql_bookmarks: &ArcSqlBookmarks) -> ArcBookmarkUpdateLog {
        sql_bookmarks.clone()
    }

    pub async fn sql_phases_factory(
        &self,
        repo_config: &ArcRepoConfig,
    ) -> Result<ArcSqlPhasesFactory> {
        let mut sql_phases_factory = self
            .open::<SqlPhasesFactory>(&repo_config.storage_config.metadata)
            .await
            .context(RepoFactoryError::Phases)?;
        if let Some(pool) = self.maybe_volatile_pool("phases")? {
            sql_phases_factory.enable_caching(self.env.fb, pool);
        }
        Ok(Arc::new(sql_phases_factory))
    }

    pub async fn bonsai_hg_mapping(
        &self,
        repo_config: &ArcRepoConfig,
    ) -> Result<ArcBonsaiHgMapping> {
        let builder = self
            .open::<SqlBonsaiHgMappingBuilder>(&repo_config.storage_config.metadata)
            .await
            .context(RepoFactoryError::BonsaiHgMapping)?;
        let bonsai_hg_mapping = builder.build(self.env.rendezvous_options);
        if let Some(pool) = self.maybe_volatile_pool("bonsai_hg_mapping")? {
            Ok(Arc::new(CachingBonsaiHgMapping::new(
                self.env.fb,
                Arc::new(bonsai_hg_mapping),
                pool,
            )))
        } else {
            Ok(Arc::new(bonsai_hg_mapping))
        }
    }

    pub async fn bonsai_git_mapping(
        &self,
        repo_config: &ArcRepoConfig,
        repo_identity: &ArcRepoIdentity,
    ) -> Result<ArcBonsaiGitMapping> {
        let bonsai_git_mapping = self
            .open::<SqlBonsaiGitMappingConnection>(&repo_config.storage_config.metadata)
            .await
            .context(RepoFactoryError::BonsaiGitMapping)?
            .with_repo_id(repo_identity.id());
        Ok(Arc::new(bonsai_git_mapping))
    }

    pub async fn long_running_requests_queue(
        &self,
        repo_config: &ArcRepoConfig,
    ) -> Result<ArcLongRunningRequestsQueue> {
        let long_running_requests_queue = self
            .open::<SqlLongRunningRequestsQueue>(&repo_config.storage_config.metadata)
            .await
            .context(RepoFactoryError::LongRunningRequestsQueue)?;
        Ok(Arc::new(long_running_requests_queue))
    }

    pub async fn bonsai_globalrev_mapping(
        &self,
        repo_config: &ArcRepoConfig,
    ) -> Result<ArcBonsaiGlobalrevMapping> {
        let bonsai_globalrev_mapping = self
            .open::<SqlBonsaiGlobalrevMapping>(&repo_config.storage_config.metadata)
            .await
            .context(RepoFactoryError::BonsaiGlobalrevMapping)?;
        if let Some(pool) = self.maybe_volatile_pool("bonsai_globalrev_mapping")? {
            Ok(Arc::new(CachingBonsaiGlobalrevMapping::new(
                self.env.fb,
                Arc::new(bonsai_globalrev_mapping),
                pool,
            )))
        } else {
            Ok(Arc::new(bonsai_globalrev_mapping))
        }
    }

    pub async fn pushrebase_mutation_mapping(
        &self,
        repo_config: &ArcRepoConfig,
    ) -> Result<ArcPushrebaseMutationMapping> {
        let conn = self
            .open::<SqlPushrebaseMutationMappingConnection>(&repo_config.storage_config.metadata)
            .await
            .context(RepoFactoryError::PushrebaseMutationMapping)?;
        Ok(Arc::new(conn.with_repo_id(repo_config.repoid)))
    }

    pub async fn repo_bonsai_svnrev_mapping(
        &self,
        repo_config: &ArcRepoConfig,
        repo_identity: &ArcRepoIdentity,
    ) -> Result<ArcRepoBonsaiSvnrevMapping> {
        let bonsai_svnrev_mapping = self
            .open::<SqlBonsaiSvnrevMapping>(&repo_config.storage_config.metadata)
            .await
            .context(RepoFactoryError::BonsaiSvnrevMapping)?;
        let bonsai_svnrev_mapping: Arc<dyn BonsaiSvnrevMapping + Send + Sync> =
            if let Some(pool) = self.maybe_volatile_pool("bonsai_svnrev_mapping")? {
                Arc::new(CachingBonsaiSvnrevMapping::new(
                    self.env.fb,
                    Arc::new(bonsai_svnrev_mapping),
                    pool,
                ))
            } else {
                Arc::new(bonsai_svnrev_mapping)
            };
        Ok(Arc::new(RepoBonsaiSvnrevMapping::new(
            repo_identity.id(),
            bonsai_svnrev_mapping,
        )))
    }

    pub async fn filenodes(
        &self,
        repo_config: &ArcRepoConfig,
        repo_identity: &ArcRepoIdentity,
    ) -> Result<ArcFilenodes> {
        let sql_factory = self
            .sql_factory(&repo_config.storage_config.metadata)
            .await?;
        let mut filenodes_builder = sql_factory
            .open_shardable::<NewFilenodesBuilder>()
            .context(RepoFactoryError::Filenodes)?;
        if let Caching::Enabled(_) = self.env.caching {
            let filenodes_tier = sql_factory.tier_info_shardable::<NewFilenodesBuilder>()?;
            let filenodes_pool = self
                .maybe_volatile_pool("filenodes")?
                .ok_or(RepoFactoryError::Filenodes)?;
            let filenodes_history_pool = self
                .maybe_volatile_pool("filenodes_history")?
                .ok_or(RepoFactoryError::Filenodes)?;
            filenodes_builder.enable_caching(
                self.env.fb,
                filenodes_pool,
                filenodes_history_pool,
                "newfilenodes",
                &filenodes_tier.tier_name,
            );
        }
        Ok(Arc::new(filenodes_builder.build(repo_identity.id())))
    }

    pub async fn hg_mutation_store(
        &self,
        repo_config: &ArcRepoConfig,
        repo_identity: &ArcRepoIdentity,
    ) -> Result<ArcHgMutationStore> {
        let sql_factory = self
            .sql_factory(&repo_config.storage_config.metadata)
            .await?;
        let hg_mutation_store = sql_factory
            .open::<SqlHgMutationStoreBuilder>()
            .context(RepoFactoryError::HgMutationStore)?
            .with_repo_id(repo_identity.id());
        Ok(Arc::new(hg_mutation_store))
    }

    pub async fn segmented_changelog(
        &self,
        repo_config: &ArcRepoConfig,
        repo_identity: &ArcRepoIdentity,
        changeset_fetcher: &ArcChangesetFetcher,
        bookmarks: &ArcBookmarks,
        repo_blobstore: &ArcRepoBlobstore,
    ) -> Result<ArcSegmentedChangelog> {
        let sql_connections = self
            .open::<SegmentedChangelogSqlConnections>(&repo_config.storage_config.metadata)
            .await
            .context(RepoFactoryError::SegmentedChangelog)?;
        let pool = self.maybe_volatile_pool("segmented_changelog")?;
        let segmented_changelog = new_server_segmented_changelog(
            self.env.fb,
            &self.ctx(Some(&repo_identity)),
            &repo_identity,
            repo_config.segmented_changelog_config.clone(),
            sql_connections,
            changeset_fetcher.clone(),
            bookmarks.clone(),
            repo_blobstore.clone(),
            pool,
        )
        .await
        .context(RepoFactoryError::SegmentedChangelog)?;
        Ok(Arc::new(segmented_changelog))
    }

    pub fn repo_derived_data(
        &self,
        repo_identity: &ArcRepoIdentity,
        repo_config: &ArcRepoConfig,
        changesets: &ArcChangesets,
        bonsai_hg_mapping: &ArcBonsaiHgMapping,
        filenodes: &ArcFilenodes,
        repo_blobstore: &ArcRepoBlobstore,
    ) -> Result<ArcRepoDerivedData> {
        let config = repo_config.derived_data_config.clone();
        let lease = lease_init(self.env.fb, self.env.caching, DERIVED_DATA_LEASE)?;
        let scuba = build_scuba(
            self.env.fb,
            config.scuba_table.clone(),
            repo_identity.name(),
        );
        let derivation_service_client =
            get_derivation_client(self.env.fb, self.env.remote_derivation_options.clone())?;
        Ok(Arc::new(RepoDerivedData::new(
            repo_identity.id(),
            repo_identity.name().to_string(),
            changesets.clone(),
            bonsai_hg_mapping.clone(),
            filenodes.clone(),
            repo_blobstore.as_ref().clone(),
            lease,
            scuba,
            config,
            derivation_service_client,
        )?))
    }

    pub async fn skiplist_index(
        &self,
        repo_config: &ArcRepoConfig,
        repo_identity: &ArcRepoIdentity,
    ) -> Result<ArcSkiplistIndex> {
        let blobstore_without_cache = self
            .repo_blobstore_from_blobstore(
                repo_identity,
                repo_config,
                &self
                    .blobstore_no_cache(&repo_config.storage_config.blobstore)
                    .await?,
            )
            .await?;
        SkiplistIndex::from_blobstore(
            &self.ctx(Some(&repo_identity)),
            &repo_config.skiplist_index_blobstore_key,
            &blobstore_without_cache.boxed(),
        )
        .await
    }

    pub async fn repo_blobstore(
        &self,
        repo_identity: &ArcRepoIdentity,
        repo_config: &ArcRepoConfig,
    ) -> Result<ArcRepoBlobstore> {
        let blobstore = self
            .blobstore(&repo_config.storage_config.blobstore)
            .await?;
        Ok(Arc::new(
            self.repo_blobstore_from_blobstore(repo_identity, repo_config, &blobstore)
                .await?,
        ))
    }

    pub fn filestore_config(&self, repo_config: &ArcRepoConfig) -> ArcFilestoreConfig {
        let filestore_config = repo_config
            .filestore
            .as_ref()
            .map(|p| FilestoreConfig {
                chunk_size: Some(p.chunk_size),
                concurrency: p.concurrency,
            })
            .unwrap_or_default();
        Arc::new(filestore_config)
    }

    pub async fn redaction_config_blobstore(&self) -> Result<ArcRedactionConfigBlobstore> {
        self.redaction_config_blobstore_from_config(&self.redaction_config.blobstore)
            .await
    }

    pub async fn repo_ephemeral_blobstore(
        &self,
        repo_identity: &ArcRepoIdentity,
        repo_config: &ArcRepoConfig,
    ) -> Result<ArcRepoEphemeralBlobstore> {
        if let Some(ephemeral_config) = &repo_config.storage_config.ephemeral_blobstore {
            let blobstore = self.blobstore(&ephemeral_config.blobstore).await?;
            let ephemeral_blobstore = RepoEphemeralBlobstoreBuilder::with_database_config(
                self.env.fb,
                &ephemeral_config.metadata,
                &self.env.mysql_options,
                self.env.readonly_storage.0,
            )?
            .build(
                repo_identity.id(),
                blobstore,
                ephemeral_config.initial_bubble_lifespan,
                ephemeral_config.bubble_expiration_grace,
            );
            Ok(Arc::new(ephemeral_blobstore))
        } else {
            Ok(Arc::new(RepoEphemeralBlobstore::disabled(
                repo_identity.id(),
            )))
        }
    }

    pub async fn mutable_renames(&self, repo_config: &ArcRepoConfig) -> Result<ArcMutableRenames> {
        let sql_store = self
            .open::<SqlMutableRenamesStore>(&repo_config.storage_config.metadata)
            .await
            .context(RepoFactoryError::MutableRenames)?;
        Ok(Arc::new(MutableRenames::new(repo_config.repoid, sql_store)))
    }

    pub fn derived_data_manager_set(
        &self,
        repo_identity: &ArcRepoIdentity,
        repo_config: &ArcRepoConfig,
        changesets: &ArcChangesets,
        bonsai_hg_mapping: &ArcBonsaiHgMapping,
        filenodes: &ArcFilenodes,
        repo_blobstore: &ArcRepoBlobstore,
    ) -> Result<ArcDerivedDataManagerSet> {
        let config = repo_config.derived_data_config.clone();
        let lease = lease_init(self.env.fb, self.env.caching, DERIVED_DATA_LEASE)?;
        let scuba = build_scuba(
            self.env.fb,
            config.scuba_table.clone(),
            repo_identity.name(),
        );
        let derivation_service_client =
            get_derivation_client(self.env.fb, self.env.remote_derivation_options.clone())?;
        anyhow::Ok(Arc::new(DerivedDataManagerSet::new(
            repo_identity.id(),
            repo_identity.name().to_string(),
            changesets.clone(),
            bonsai_hg_mapping.clone(),
            filenodes.clone(),
            repo_blobstore.as_ref().clone(),
            lease,
            scuba,
            config,
            derivation_service_client,
        )?))
    }
}

fn lease_init(
    fb: FacebookInit,
    caching: Caching,
    lease_type: &'static str,
) -> Result<Arc<dyn LeaseOps>> {
    // Derived data leasing is performed through the cache, so is only
    // available if caching is enabled.
    if let Caching::Enabled(_) = caching {
        Ok(Arc::new(MemcacheOps::new(fb, lease_type, "")?))
    } else {
        Ok(Arc::new(InProcessLease::new()))
    }
}

fn build_scuba(
    fb: FacebookInit,
    scuba_table: Option<String>,
    reponame: &str,
) -> MononokeScubaSampleBuilder {
    let mut scuba = MononokeScubaSampleBuilder::with_opt_table(fb, scuba_table);
    scuba.add_common_server_data();
    scuba.add("reponame", reponame);
    scuba
}

fn get_derivation_client(
    fb: FacebookInit,
    remote_derivation_options: RemoteDerivationOptions,
) -> Result<Option<Arc<dyn DerivationClient<Output = ()>>>> {
    let derivation_service_client: Option<Arc<dyn DerivationClient<Output = ()>>> =
        if remote_derivation_options.derive_remotely {
            #[cfg(fbcode_build)]
            {
                let client = match remote_derivation_options.smc_tier {
                    Some(smc_tier) => DerivationServiceClient::from_tier_name(fb, smc_tier)?,
                    None => DerivationServiceClient::new(fb)?,
                };
                Some(Arc::new(client))
            }
            #[cfg(not(fbcode_build))]
            {
                let _ = fb;
                None
            }
        } else {
            None
        };
    Ok(derivation_service_client)
}
