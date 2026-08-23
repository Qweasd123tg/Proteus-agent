use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use proteus_process_host::{
    DEFAULT_MAX_FRAME_BYTES, DEFAULT_MAX_QUEUED_CONTROL_WRITE_BYTES,
    DEFAULT_MAX_QUEUED_CONTROL_WRITES, DEFAULT_MAX_QUEUED_WRITE_BYTES, DEFAULT_MAX_QUEUED_WRITES,
    ProcessTransportLimits, ReceiveLimits,
};

pub const DEFAULT_V3_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_V3_CANCEL_GRACE: Duration = Duration::from_millis(250);

/// Host-owned bounds for one configured component. None of these limits may
/// vary by `module_id`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ComponentBrokerOptions {
    pub handshake_timeout: Duration,
    pub cancel_grace: Duration,
    pub receive_limits: ReceiveLimits,
    pub notification_limits: ReceiveLimits,
    pub data_writer_capacity: usize,
    pub control_writer_capacity: usize,
    pub max_outbound_frame_bytes: usize,
    pub data_writer_byte_capacity: usize,
    pub control_writer_byte_capacity: usize,
    pub root_command_capacity: usize,
    pub control_command_capacity: usize,
    pub max_active_roots: usize,
    pub max_pending_roots: usize,
    pub max_active_total: usize,
    pub reserved_nested: usize,
    pub max_active_nested: usize,
    pub max_callback_depth: usize,
    pub max_callbacks_per_root: usize,
    pub max_pending_callbacks: usize,
    pub max_callback_ids_per_generation: usize,
}

impl Default for ComponentBrokerOptions {
    fn default() -> Self {
        Self {
            handshake_timeout: DEFAULT_V3_HANDSHAKE_TIMEOUT,
            cancel_grace: DEFAULT_V3_CANCEL_GRACE,
            receive_limits: ReceiveLimits::new(4096, 32 * 1024 * 1024),
            notification_limits: ReceiveLimits::new(64, 4 * 1024 * 1024),
            data_writer_capacity: DEFAULT_MAX_QUEUED_WRITES,
            control_writer_capacity: DEFAULT_MAX_QUEUED_CONTROL_WRITES,
            max_outbound_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            data_writer_byte_capacity: DEFAULT_MAX_QUEUED_WRITE_BYTES,
            control_writer_byte_capacity: DEFAULT_MAX_QUEUED_CONTROL_WRITE_BYTES,
            root_command_capacity: 64,
            control_command_capacity: 256,
            max_active_roots: 16,
            max_pending_roots: 64,
            max_active_total: 32,
            reserved_nested: 16,
            max_active_nested: 16,
            max_callback_depth: 16,
            max_callbacks_per_root: 256,
            max_pending_callbacks: 256,
            max_callback_ids_per_generation: 65_536,
        }
    }
}

impl ComponentBrokerOptions {
    pub(crate) fn validate(self) -> Result<()> {
        if self.handshake_timeout.is_zero() {
            bail!("component-v3 handshake timeout must be greater than zero");
        }
        if self.cancel_grace.is_zero() {
            bail!("component-v3 cancel grace must be greater than zero");
        }
        if Instant::now().checked_add(self.cancel_grace).is_none() {
            bail!("component-v3 cancel grace exceeds the platform instant range");
        }
        validate_receive("receive", self.receive_limits)?;
        validate_receive("notification", self.notification_limits)?;
        for (label, value) in [
            ("data_writer_capacity", self.data_writer_capacity),
            ("control_writer_capacity", self.control_writer_capacity),
            ("max_outbound_frame_bytes", self.max_outbound_frame_bytes),
            ("data_writer_byte_capacity", self.data_writer_byte_capacity),
            (
                "control_writer_byte_capacity",
                self.control_writer_byte_capacity,
            ),
            ("root_command_capacity", self.root_command_capacity),
            ("control_command_capacity", self.control_command_capacity),
            ("max_active_roots", self.max_active_roots),
            ("max_pending_roots", self.max_pending_roots),
            ("max_active_total", self.max_active_total),
            ("reserved_nested", self.reserved_nested),
            ("max_active_nested", self.max_active_nested),
            ("max_callback_depth", self.max_callback_depth),
            ("max_callbacks_per_root", self.max_callbacks_per_root),
            ("max_pending_callbacks", self.max_pending_callbacks),
            (
                "max_callback_ids_per_generation",
                self.max_callback_ids_per_generation,
            ),
        ] {
            if value == 0 {
                bail!("component-v3 {label} must be greater than zero");
            }
        }
        if self.max_pending_roots < self.max_active_roots {
            bail!("component-v3 max_pending_roots must cover max_active_roots");
        }
        if self.max_active_total <= self.reserved_nested {
            bail!("component-v3 max_active_total must exceed reserved_nested");
        }
        if self.max_active_roots > self.max_active_total - self.reserved_nested {
            bail!("component-v3 roots must preserve reserved_nested capacity");
        }
        if self.max_active_nested > self.reserved_nested {
            bail!("component-v3 max_active_nested must fit reserved_nested capacity");
        }
        if self.max_callback_depth > self.max_active_nested {
            bail!(
                "component-v3 max_active_nested must cover max_callback_depth to avoid nested admission deadlock"
            );
        }
        Ok(())
    }

    pub(crate) fn transport_limits(self) -> ProcessTransportLimits {
        ProcessTransportLimits::new(self.receive_limits, self.data_writer_capacity)
            .with_control_queue(self.control_writer_capacity)
            .with_write_byte_limits(
                self.max_outbound_frame_bytes,
                self.data_writer_byte_capacity,
                self.control_writer_byte_capacity,
            )
    }
}

fn validate_receive(label: &str, limits: ReceiveLimits) -> Result<()> {
    if limits.max_buffered_frames() == 0 {
        bail!("component-v3 {label} frame limit must be greater than zero");
    }
    if limits.max_buffered_bytes() == 0 {
        bail!("component-v3 {label} byte limit must be greater than zero");
    }
    Ok(())
}
