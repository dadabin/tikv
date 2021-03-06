// Copyright 2016 PingCAP, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// See the License for the specific language governing permissions and
// limitations under the License.

mod checksum;
pub mod codec;
mod dag;
mod endpoint;
mod error;
pub mod local_metrics;
mod metrics;
mod readpool_context;
mod statistics;
mod tracker;
mod util;

pub use self::endpoint::err_resp;
pub use self::error::{Error, Result};
pub use self::readpool_context::Context as ReadPoolContext;

use std::time::Duration;

use kvproto::{coprocessor as coppb, kvrpcpb};

use util::time::Instant;

pub const REQ_TYPE_DAG: i64 = 103;
pub const REQ_TYPE_ANALYZE: i64 = 104;
pub const REQ_TYPE_CHECKSUM: i64 = 105;

const SINGLE_GROUP: &[u8] = b"SingleGroup";

type HandlerStreamStepResult = Result<(Option<coppb::Response>, bool)>;

trait RequestHandler: Send {
    fn handle_request(&mut self) -> Result<coppb::Response> {
        panic!("unary request is not supported for this handler");
    }

    fn handle_streaming_request(&mut self) -> HandlerStreamStepResult {
        panic!("streaming request is not supported for this handler");
    }

    fn collect_metrics_into(&mut self, _metrics: &mut self::dag::executor::ExecutorMetrics) {
        // Do nothing by default
    }

    fn into_boxed(self) -> Box<RequestHandler>
    where
        Self: 'static + Sized,
    {
        box self
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Deadline {
    /// Used to construct the Error when deadline exceeded
    tag: &'static str,

    start_time: Instant,
    deadline: Instant,
}

impl Deadline {
    /// Initialize a deadline that counting from current.
    pub fn from_now(tag: &'static str, after_duration: Duration) -> Self {
        let start_time = Instant::now_coarse();
        let deadline = start_time + after_duration;
        Self {
            tag,
            start_time,
            deadline,
        }
    }

    /// Reset deadline according to the newly specified duration.
    // TODO: Remove it in read pool PR. Since we can construct a precise deadline.
    pub fn reset(&mut self, after_duration: Duration) {
        self.deadline = self.start_time + after_duration;
    }

    /// Returns error if the deadline is exceeded.
    pub fn check_if_exceeded(&self) -> Result<()> {
        let now = Instant::now_coarse();
        if self.deadline <= now {
            let elapsed = now.duration_since(self.start_time);
            return Err(Error::Outdated(elapsed, self.tag));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct ReqContext {
    /// The tag of the request
    pub tag: &'static str,

    /// The rpc context carried in the request
    pub context: kvrpcpb::Context,

    /// The first range of the request
    pub first_range: Option<coppb::KeyRange>,

    /// The length of the range
    pub ranges_len: usize,

    /// The deadline of the request
    pub deadline: Deadline,

    /// The peer address of the request
    pub peer: Option<String>,

    /// Whether the request is a descending scan (only applicable to DAG)
    pub is_desc_scan: Option<bool>,

    /// The transaction start_ts of the request
    pub txn_start_ts: Option<u64>,
}

impl ReqContext {
    pub fn new(
        tag: &'static str,
        context: kvrpcpb::Context,
        ranges: &[coppb::KeyRange],
        peer: Option<String>,
        is_desc_scan: Option<bool>,
        txn_start_ts: Option<u64>,
    ) -> Self {
        let deadline = Deadline::from_now(tag, Duration::from_secs(0));
        Self {
            tag,
            context,
            deadline,
            peer,
            is_desc_scan,
            txn_start_ts,
            first_range: ranges.first().cloned(),
            ranges_len: ranges.len(),
        }
    }

    // TODO: Remove it in read pool PR. Since we can construct a precise deadline.
    pub fn set_max_handle_duration(&mut self, request_max_handle_duration: Duration) {
        self.deadline.reset(request_max_handle_duration)
    }

    #[cfg(test)]
    pub fn default_for_test() -> Self {
        Self::new("test", kvrpcpb::Context::new(), &[], None, None, None)
    }
}

pub use self::dag::{ScanOn, Scanner};
pub use self::endpoint::{
    Host as EndPointHost, RequestTask, Task as EndPointTask, DEFAULT_REQUEST_MAX_HANDLE_SECS,
};
