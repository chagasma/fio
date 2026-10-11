use std::{fs, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use directories::ProjectDirs;
use rusqlite::{Connection, params};
use serde::Serialize;

use crate::store::{catalog::Catalog, fs::read_dir_sorted};

const MAX_HITS: usize = 10_000;

#[derive(Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HitKind {
    Demand,
    Session,
}

impl HitKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Demand => "demand",
            Self::Session => "session",
        }
    }

    fn from_name(name: &str) -> Self {
        if name == "session" {
            Self::Session
        } else {
            Self::Demand
        }
    }
}

#[derive(Clone, Serialize)]
pub struct SearchHit {
    pub workspace: PathBuf,
    pub project: String,
    pub demand: String,
    pub title: String,
    pub kind: HitKind,
    pub path: PathBuf,
    pub snippet: String,
}

pub struct Index(Connection);

impl Index {
    pub fn rebuild(catalog: &Catalog) -> Result<Self> {
        let dirs =
            ProjectDirs::from("", "", "fio").context("Could not locate the cache directory")?;
        fs::create_dir_all(dirs.cache_dir())?;
        let mut connection = Connection::open(dirs.cache_dir().join("index.db"))?;
        connection.busy_timeout(Duration::from_secs(5))?;
        let transaction = connection.transaction()?;
        transaction.execute_batch("DROP TABLE IF EXISTS documents; CREATE VIRTUAL TABLE documents USING fts5(workspace UNINDEXED, project, demand, title, kind UNINDEXED, path UNINDEXED, content, tokenize='unicode61 remove_diacritics 2');")?;
        {
            let mut insert =
                transaction.prepare("INSERT INTO documents VALUES (?1,?2,?3,?4,?5,?6,?7)")?;
            for entry in &catalog.demands {
                let loaded = &entry.loaded;
                let demand = &loaded.document.metadata;
                let mut add = |kind: HitKind, path: &std::path::Path, content: &str| {
                    insert.execute(params![
                        entry.workspace.to_string_lossy(),
                        demand.project,
                        demand.id.as_str(),
                        demand.title,
                        kind.name(),
                        path.to_string_lossy(),
                        content
                    ])
                };
                add(HitKind::Demand, &loaded.path, &loaded.document.render())?;
                // A session that cannot be read does not prevent indexing the rest.
                for path in read_dir_sorted(&loaded.folder().join("sessions"))? {
                    if path.extension().is_some_and(|extension| extension == "md")
                        && let Ok(content) = fs::read_to_string(&path)
                    {
                        add(HitKind::Session, &path, &content)?;
                    }
                }
            }
        }
        transaction.commit()?;
        Ok(Self(connection))
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let query = query
            .split(|c: char| !c.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .map(|word| format!("\"{word}\"*"))
            .collect::<Vec<_>>()
            .join(" AND ");
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let mut statement = self.0.prepare("SELECT workspace,project,demand,title,kind,path,snippet(documents,6,'[',']',' … ',24) FROM documents WHERE documents MATCH ?1 ORDER BY bm25(documents,0,1,2,5,0,0,1),project,demand,path LIMIT ?2")?;
        let rows = statement.query_map(
            params![
                query,
                i64::try_from(limit.min(MAX_HITS)).unwrap_or(i64::MAX)
            ],
            |row| {
                Ok(SearchHit {
                    workspace: PathBuf::from(row.get::<_, String>(0)?),
                    project: row.get(1)?,
                    demand: row.get(2)?,
                    title: row.get(3)?,
                    kind: HitKind::from_name(&row.get::<_, String>(4)?),
                    path: PathBuf::from(row.get::<_, String>(5)?),
                    snippet: row.get(6)?,
                })
            },
        )?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
