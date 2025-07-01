/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This source code is dual-licensed under either the MIT license found in the
 * LICENSE-MIT file in the root directory of this source tree or the Apache
 * License, Version 2.0 found in the LICENSE-APACHE file in the root directory
 * of this source tree. You may select, at your option, one of the
 * above-listed licenses.
 */

use std::fmt::Display;

use futures::Poll;
use futures::Stream;

/// "Context" support for streams.
pub trait StreamErrorContext: Stream + Sized {
    /// Add context to the error returned by this stream
    fn context<D>(self, context: D) -> ContextStream<Self, D>
    where
        D: Display + Clone + Send + Sync + 'static;

    /// Add context created by provided function to the error returned by this stream
    fn with_context<D, F>(self, f: F) -> WithContextStream<Self, F>
    where
        D: Display + Clone + Send + Sync + 'static,
        F: FnMut() -> D;
}

impl<S, E> StreamErrorContext for S
where
    S: Stream<Error = E> + Sized,
    E: Into<anyhow::Error>,
{
    fn context<D>(self, displayable: D) -> ContextStream<Self, D>
    where
        D: Display + Clone + Send + Sync + 'static,
    {
        ContextStream::new(self, displayable)
    }

    fn with_context<D, F>(self, f: F) -> WithContextStream<Self, F>
    where
        D: Display + Clone + Send + Sync + 'static,
        F: FnMut() -> D,
    {
        WithContextStream::new(self, f)
    }
}

pub struct ContextStream<A, D> {
    inner: A,
    displayable: D,
}

impl<A, D> ContextStream<A, D> {
    fn new(stream: A, displayable: D) -> Self {
        Self {
            inner: stream,
            displayable,
        }
    }
}

impl<A, E, D> Stream for ContextStream<A, D>
where
    A: Stream<Error = E>,
    E: Into<anyhow::Error>,
    D: Display + Clone + Send + Sync + 'static,
{
    type Item = A::Item;
    type Error = anyhow::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        match self.inner.poll() {
            Err(err) => Err(err.into().context(self.displayable.clone())),
            Ok(item) => Ok(item),
        }
    }
}

pub struct WithContextStream<A, F> {
    inner: A,
    displayable: F,
}

impl<A, F> WithContextStream<A, F> {
    fn new(stream: A, displayable: F) -> Self {
        Self {
            inner: stream,
            displayable,
        }
    }
}

impl<A, E, F, D> Stream for WithContextStream<A, F>
where
    A: Stream<Error = E>,
    E: Into<anyhow::Error>,
    D: Display + Clone + Send + Sync + 'static,
    F: FnMut() -> D,
{
    type Item = A::Item;
    type Error = anyhow::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        match self.inner.poll() {
            Err(err) => {
                let context = (self.displayable)();
                Err(err.into().context(context))
            }
            Ok(item) => Ok(item),
        }
    }
}

#[cfg(test)]
mod test {
    use anyhow::format_err;
    use futures::stream::iter_result;

    use super::*;

    #[test]
    fn stream_poll_after_completion_fail() {
        let stream = iter_result(vec![
            Ok(17),
            Err(format_err!("foo").context("bar")),
            Err(format_err!("baz").context("wiggle")),
        ]);
        let mut stream = stream.context("foo");
        let _ = stream.poll();
        let _ = stream.poll();
        let _ = stream.poll();
    }

    #[test]
    fn stream_poll_after_completion_fail_with_context() {
        let stream = iter_result(vec![
            Ok(17),
            Err(format_err!("foo").context("bar")),
            Err(format_err!("baz").context("wiggle")),
        ]);
        let mut stream = stream.with_context(|| "foo");
        let _ = stream.poll();
        let _ = stream.poll();
        let _ = stream.poll();
    }

    #[test]
    fn stream_poll_after_completion_error() {
        let stream = iter_result(vec![
            Ok(17),
            Err(format_err!("bar")),
            Err(format_err!("baz")),
        ]);
        let mut stream = stream.context("foo");
        let _ = stream.poll();
        let _ = stream.poll();
        let _ = stream.poll();
    }

    #[test]
    fn stream_poll_after_completion_error_with_context() {
        let stream = iter_result(vec![
            Ok(17),
            Err(format_err!("bar")),
            Err(format_err!("baz")),
        ]);
        let mut stream = stream.with_context(|| "foo");
        let _ = stream.poll();
        let _ = stream.poll();
        let _ = stream.poll();
    }
}
