//! Bounded resident-process pool and resumable-task index for the process
//! subagent runner.
//!
//! One mutex owner keeps idle/reserved/leased state and `task_id` bindings
//! coherent. Resume preparation removes its child from the eviction set before
//! the detached execution waits for a role permit, so a concurrent fresh turn
//! cannot clear and reuse that process in between prepare and lease.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};

use super::{child::ChildProcess, config::ProcessRoleConfig};
use crate::domain::SessionId;

pub(super) struct ProcessPool {
    idle: Vec<IdleChild>,
    reserved: HashMap<u64, ReservedChild>,
    leased: HashSet<u64>,
    resumable: HashMap<String, ResumableProcessTask>,
    next_process_id: u64,
    clock: u64,
    max_idle_processes: usize,
}

pub(super) struct PooledChild {
    pub child: ChildProcess,
    pub cwd: PathBuf,
    pub id: u64,
    pub role: String,
    pub used: bool,
}

struct IdleChild {
    child: PooledChild,
    last_used: u64,
}

struct ReservedChild {
    child: PooledChild,
    task_id: String,
}

#[derive(Debug, Clone)]
struct ResumableProcessTask {
    session_id: SessionId,
    role: String,
    process_id: u64,
    cwd: PathBuf,
}

#[derive(Debug, Clone)]
pub(super) struct ResumeReservation {
    process_id: u64,
    task_id: String,
}

pub(super) struct ReleaseOutcome {
    pub retained: bool,
    pub evicted: Vec<PooledChild>,
}

impl ProcessPool {
    pub(super) fn new(max_idle_processes: usize) -> Self {
        Self {
            idle: Vec::new(),
            reserved: HashMap::new(),
            leased: HashSet::new(),
            resumable: HashMap::new(),
            next_process_id: 0,
            clock: 0,
            max_idle_processes,
        }
    }

    /// Atomically validates and removes a resume target from the idle
    /// eviction set. The child remains reserved until execution acquires its
    /// role permit or the pending launch is cancelled/rolled back.
    pub(super) fn reserve_resume(
        &mut self,
        task_id: &str,
        session_id: SessionId,
        role: &str,
        cwd: &Path,
    ) -> Result<ResumeReservation> {
        let requested_cwd = canonical_cwd(cwd);
        let task = self
            .resumable
            .get(task_id)
            .cloned()
            .ok_or_else(unknown_task_id)?;
        if task.session_id != session_id {
            bail!("unknown task_id (expired or from another session)");
        }
        if task.role != role {
            bail!(
                "task_id belongs to subagent role {}, but request role is {role}",
                task.role
            );
        }
        if task.cwd != requested_cwd {
            bail!("task_id belongs to a different workspace");
        }

        if let Some(index) = self
            .idle
            .iter()
            .position(|entry| entry.child.id == task.process_id)
        {
            let mut entry = self.idle.swap_remove(index).child;
            if !entry.child.is_alive() {
                self.purge_process(task.process_id);
                bail!("unknown task_id (subagent child process exited)");
            }
            self.reserved.insert(
                task.process_id,
                ReservedChild {
                    child: entry,
                    task_id: task_id.to_owned(),
                },
            );
            return Ok(ResumeReservation {
                process_id: task.process_id,
                task_id: task_id.to_owned(),
            });
        }
        if self.reserved.contains_key(&task.process_id) {
            bail!("task_id belongs to a subagent resume that is still starting");
        }
        if self.leased.contains(&task.process_id) {
            bail!("task_id belongs to a subagent that is still running; wait for it first");
        }

        self.purge_process(task.process_id);
        bail!("unknown task_id (subagent child process was restarted)")
    }

    pub(super) fn lease_reserved(
        &mut self,
        reservation: &ResumeReservation,
    ) -> Result<PooledChild> {
        let reserved = self
            .reserved
            .remove(&reservation.process_id)
            .ok_or_else(|| anyhow!("subagent resume reservation was lost"))?;
        if reserved.task_id != reservation.task_id {
            self.purge_process(reservation.process_id);
            bail!("subagent resume reservation changed before lease");
        }
        let mut child = reserved.child;
        if !child.child.is_alive() {
            self.purge_process(child.id);
            bail!("unknown task_id (subagent child process exited)");
        }
        self.leased.insert(child.id);
        Ok(child)
    }

    /// Rolls back a prepared resume that never reached a child turn. The
    /// previous task binding survives only if the child remains inside the
    /// bounded idle set.
    pub(super) fn cancel_reservation(
        &mut self,
        reservation: &ResumeReservation,
    ) -> Result<ReleaseOutcome> {
        let reserved = self
            .reserved
            .remove(&reservation.process_id)
            .ok_or_else(|| anyhow!("subagent resume reservation was lost"))?;
        if reserved.task_id != reservation.task_id {
            self.purge_process(reservation.process_id);
            bail!("subagent resume reservation changed before rollback");
        }
        let mut evicted = Vec::new();
        self.push_idle(reserved.child, &mut evicted);
        Ok(ReleaseOutcome {
            retained: self.resumable.contains_key(&reservation.task_id),
            evicted,
        })
    }

    /// Leases a same-role/same-cwd idle process for a fresh task or spawns a
    /// new one. Reserved and leased children are never considered.
    pub(super) fn lease_fresh(
        &mut self,
        binary: &Path,
        role_name: &str,
        role: &ProcessRoleConfig,
        cwd: &Path,
    ) -> Result<PooledChild> {
        let cwd_key = canonical_cwd(cwd);
        let mut index = 0;
        while index < self.idle.len() {
            if !self.idle[index].child.child.is_alive() {
                let dead = self.idle.swap_remove(index).child;
                self.purge_process(dead.id);
                continue;
            }
            if self.idle[index].child.role == role_name && self.idle[index].child.cwd == cwd_key {
                let child = self.idle.swap_remove(index).child;
                self.leased.insert(child.id);
                return Ok(child);
            }
            index += 1;
        }

        let id = self.next_process_id;
        self.next_process_id = self.next_process_id.wrapping_add(1);
        let child = ChildProcess::spawn(binary, &role.config, &role.args, cwd)
            .with_context(|| format!("failed to spawn process-subagent role {role_name}"))?;
        self.leased.insert(id);
        Ok(PooledChild {
            id,
            child,
            cwd: cwd_key,
            role: role_name.to_owned(),
            used: false,
        })
    }

    /// A successful fresh `ClearHistory` invalidates every older task bound
    /// to that process before the new turn is sent.
    pub(super) fn invalidate_history(&mut self, process_id: u64) {
        self.purge_process(process_id);
    }

    /// Returns a terminal child to the bounded idle pool and installs exactly
    /// one resumable task binding for its current history. The returned flag is
    /// authoritative: cap pressure may immediately evict this same child.
    pub(super) fn release(
        &mut self,
        child: PooledChild,
        alive: bool,
        session_id: SessionId,
        task_id: String,
    ) -> ReleaseOutcome {
        self.leased.remove(&child.id);
        self.purge_process(child.id);
        if !alive {
            return ReleaseOutcome {
                retained: false,
                evicted: vec![child],
            };
        }

        let process_id = child.id;
        self.resumable.insert(
            task_id.clone(),
            ResumableProcessTask {
                session_id,
                role: child.role.clone(),
                process_id,
                cwd: child.cwd.clone(),
            },
        );
        let mut evicted = Vec::new();
        self.push_idle(child, &mut evicted);
        ReleaseOutcome {
            retained: self.resumable.contains_key(&task_id),
            evicted,
        }
    }

    pub(super) fn discard(&mut self, child: PooledChild) {
        self.leased.remove(&child.id);
        self.purge_process(child.id);
    }

    fn push_idle(&mut self, child: PooledChild, evicted: &mut Vec<PooledChild>) {
        self.clock = self.clock.saturating_add(1);
        self.idle.push(IdleChild {
            child,
            last_used: self.clock,
        });
        while self.idle.len() > self.max_idle_processes {
            let Some(index) = self
                .idle
                .iter()
                .enumerate()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(index, _)| index)
            else {
                break;
            };
            let victim = self.idle.swap_remove(index).child;
            self.purge_process(victim.id);
            evicted.push(victim);
        }
    }

    fn purge_process(&mut self, process_id: u64) {
        self.resumable
            .retain(|_, task| task.process_id != process_id);
    }
}

fn canonical_cwd(cwd: &Path) -> PathBuf {
    std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf())
}

fn unknown_task_id() -> anyhow::Error {
    anyhow!("unknown task_id (expired or from another session)")
}
