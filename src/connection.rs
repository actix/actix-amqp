use std::{collections::VecDeque, io, pin::Pin, task::Context, task::Poll, time::Duration};

use either::Either;
use futures::{future, Future, Stream};
use ntex::channel::oneshot;
use ntex::task::LocalWaker;
use ntex::util::time::LowResTimeService;
use ntex_codec::{AsyncRead, AsyncWrite, Framed};

use ntex_amqp_codec::protocol::{Begin, Close, End, Error, Frame};
use ntex_amqp_codec::{AHashMap, AmqpCodec, AmqpCodecError, AmqpFrame};

use crate::cell::{Cell, WeakCell};
use crate::errors::AmqpTransportError;
use crate::hb::{Heartbeat, HeartbeatAction};
use crate::session::{Session, SessionInner};
use crate::Configuration;

pub struct Connection<T> {
    inner: Cell<ConnectionInner>,
    framed: Framed<T, AmqpCodec<AmqpFrame>>,
    hb: Heartbeat,
}

pub(crate) enum ChannelState {
    Opening(Option<oneshot::Sender<Session>>, WeakCell<ConnectionInner>),
    Established(Cell<SessionInner>),
    #[allow(dead_code)]
    Closing(Option<oneshot::Sender<Result<(), AmqpTransportError>>>),
}

impl ChannelState {
    fn is_opening(&self) -> bool {
        matches!(self, ChannelState::Opening(_, _))
    }
}

pub(crate) struct ConnectionInner {
    local: Configuration,
    remote: Configuration,
    write_queue: VecDeque<AmqpFrame>,
    write_task: LocalWaker,
    sessions: slab::Slab<ChannelState>,
    sessions_map: AHashMap<u16, usize>,
    error: Option<AmqpTransportError>,
    state: State,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Normal,
    Closing,
    RemoteClose,
    Drop,
}

impl<T: AsyncRead + AsyncWrite + Unpin> Connection<T> {
    pub fn new(
        framed: Framed<T, AmqpCodec<AmqpFrame>>,
        local: Configuration,
        remote: Configuration,
        time: Option<LowResTimeService>,
    ) -> Connection<T> {
        Connection {
            framed,
            hb: Heartbeat::new(
                local.timeout().unwrap(),
                remote.timeout(),
                time.unwrap_or_else(|| LowResTimeService::with(Duration::from_secs(1))),
            ),
            inner: Cell::new(ConnectionInner::new(local, remote)),
        }
    }

    pub(crate) fn new_server(
        framed: Framed<T, AmqpCodec<AmqpFrame>>,
        inner: Cell<ConnectionInner>,
        time: Option<LowResTimeService>,
    ) -> Connection<T> {
        let l_timeout = inner.get_ref().local.timeout().unwrap();
        let r_timeout = inner.get_ref().remote.timeout();
        Connection {
            framed,
            inner,
            hb: Heartbeat::new(
                l_timeout,
                r_timeout,
                time.unwrap_or_else(|| LowResTimeService::with(Duration::from_secs(1))),
            ),
        }
    }

    /// Connection controller
    pub fn controller(&self) -> ConnectionController {
        ConnectionController(self.inner.clone())
    }

    /// Get remote configuration
    pub fn remote_config(&self) -> &Configuration {
        &self.inner.get_ref().remote
    }

    /// Gracefully close connection
    pub fn close(&mut self) -> impl Future<Output = Result<(), AmqpTransportError>> {
        future::ok(())
    }

    // TODO: implement
    /// Close connection with error
    pub fn close_with_error(
        &mut self,
        _err: Error,
    ) -> impl Future<Output = Result<(), AmqpTransportError>> {
        future::ok(())
    }

    /// Opens the session
    pub fn open_session(&mut self) -> impl Future<Output = Result<Session, AmqpTransportError>> {
        let cell = self.inner.downgrade();
        let inner = self.inner.clone();

        async move {
            let inner = inner.get_mut();

            if let Some(ref e) = inner.error {
                log::error!("Connection is in error state: {:?}", e);
                Err(e.clone())
            } else {
                let (tx, rx) = oneshot::channel();

                let entry = inner.sessions.vacant_entry();
                let token = entry.key();

                if token >= inner.local.channel_max {
                    log::trace!("Too many channels: {:?}", token);
                    Err(AmqpTransportError::TooManyChannels)
                } else {
                    entry.insert(ChannelState::Opening(Some(tx), cell));

                    let begin = Begin {
                        remote_channel: None,
                        next_outgoing_id: 1,
                        incoming_window: std::u32::MAX,
                        outgoing_window: std::u32::MAX,
                        handle_max: std::u32::MAX,
                        offered_capabilities: None,
                        desired_capabilities: None,
                        properties: None,
                    };
                    inner.post_frame(AmqpFrame::new(token as u16, begin.into()));

                    rx.await.map_err(|_| AmqpTransportError::Disconnected)
                }
            }
        }
    }

    /// Get session by remote id. This method panics if session does not exists or in opening/closing state.
    pub(crate) fn get_remote_session(&self, id: usize) -> Option<Cell<SessionInner>> {
        let inner = self.inner.get_ref();
        inner.sessions_map.get(&(id as u16)).and_then(|token| {
            inner.sessions.get(*token).and_then(|channel| {
                if let ChannelState::Established(ref session) = channel {
                    Some(session.clone())
                } else {
                    None
                }
            })
        })
    }

    pub(crate) fn register_remote_session(&mut self, channel_id: u16, begin: &Begin) {
        trace!("remote session opened: {:?}", channel_id);

        let cell = self.inner.clone();
        let inner = self.inner.get_mut();
        let entry = inner.sessions.vacant_entry();
        let token = entry.key();

        let session = Cell::new(SessionInner::new(
            token,
            false,
            ConnectionController(cell),
            token as u16,
            begin.next_outgoing_id(),
            begin.incoming_window(),
            begin.outgoing_window(),
        ));
        entry.insert(ChannelState::Established(session));
        inner.sessions_map.insert(channel_id, token);

        let begin = Begin {
            remote_channel: Some(channel_id),
            next_outgoing_id: 1,
            incoming_window: std::u32::MAX,
            outgoing_window: begin.incoming_window(),
            handle_max: std::u32::MAX,
            offered_capabilities: None,
            desired_capabilities: None,
            properties: None,
        };
        inner.post_frame(AmqpFrame::new(token as u16, begin.into()));
    }

    pub(crate) fn register_write_task(&self, cx: &mut Context<'_>) {
        self.inner.write_task.register(cx.waker());
    }

    pub(crate) fn poll_outgoing(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), Either<AmqpCodecError, io::Error>>> {
        let inner = self.inner.get_mut();
        let mut update = false;
        loop {
            while !self.framed.is_write_buf_full() {
                if let Some(frame) = inner.pop_next_frame() {
                    #[cfg(feature = "frame-trace")]
                    trace!("outgoing: {:#?}", frame);
                    update = true;
                    if let Err(e) = self.framed.write(frame) {
                        error!("Cannot encode frame: {:?}", e);
                        inner.set_error(AmqpTransportError::Disconnected);
                        return Poll::Ready(Err(Either::Left(e)));
                    }
                } else {
                    break;
                }
            }

            if !self.framed.is_write_buf_empty() {
                match self.framed.flush(cx) {
                    Poll::Pending => break,
                    Poll::Ready(Err(e)) => {
                        trace!("Cannot send data: {}", e);
                        inner.set_error(AmqpTransportError::Disconnected);
                        return Poll::Ready(Err(Either::Right(e)));
                    }
                    Poll::Ready(_) => (),
                }
            } else {
                break;
            }
        }
        self.hb.update_remote(update);

        if inner.state == State::Drop
            || (inner.state == State::RemoteClose
                && inner.write_queue.is_empty()
                && self.framed.is_write_buf_empty())
        {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }

    pub(crate) fn poll_incoming(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<AmqpFrame, Either<AmqpCodecError, io::Error>>>> {
        let inner = self.inner.get_mut();

        let mut update = false;
        loop {
            match Pin::new(&mut self.framed).poll_next(cx) {
                Poll::Ready(Some(Ok(frame))) => {
                    #[cfg(feature = "frame-trace")]
                    trace!("incoming: {:#?}", frame);

                    update = true;

                    if let Frame::Empty = frame.performative() {
                        self.hb.update_local(update);
                        continue;
                    }

                    // handle connection close
                    if let Frame::Close(ref close) = frame.performative() {
                        inner.set_error(AmqpTransportError::Closed(close.error.clone()));

                        if inner.state == State::Closing {
                            log::trace!("Connection closed: {:?}", close);
                            inner.set_error(AmqpTransportError::Disconnected);
                        } else {
                            log::trace!("Connection closed remotely: {:?}", close);
                            let close = Close { error: None };
                            inner.post_frame(AmqpFrame::new(0, close.into()));
                            inner.state = State::RemoteClose;
                        }
                        return Poll::Ready(None);
                    }

                    if inner.error.is_some() {
                        error!("Connection closed but new framed is received: {:?}", frame);
                        return Poll::Ready(None);
                    }

                    // get local session id
                    let channel_id =
                        if let Some(token) = inner.sessions_map.get(&frame.channel_id()) {
                            *token
                        } else {
                            // we dont have channel info, only Begin frame is allowed on new channel
                            if let Frame::Begin(ref begin) = frame.performative() {
                                if begin.remote_channel().is_some() {
                                    inner.complete_session_creation(frame.channel_id(), begin);
                                } else {
                                    return Poll::Ready(Some(Ok(frame)));
                                }
                            } else {
                                warn!("Unexpected frame: {:#?}", frame);
                            }
                            continue;
                        };

                    // handle session frames
                    if let Some(channel) = inner.sessions.get_mut(channel_id) {
                        match channel {
                            ChannelState::Opening(_, _) => {
                                error!("Unexpected opening state: {}", channel_id);
                            }
                            ChannelState::Established(ref mut session) => {
                                match frame.performative() {
                                    Frame::Attach(attach) => {
                                        let cell = session.clone();
                                        if !session.get_mut().handle_attach(attach, cell) {
                                            return Poll::Ready(Some(Ok(frame)));
                                        }
                                    }
                                    Frame::Flow(_) | Frame::Detach(_) => {
                                        return Poll::Ready(Some(Ok(frame)));
                                    }
                                    Frame::End(remote_end) => {
                                        trace!("Remote session end: {}", frame.channel_id());
                                        let end = End { error: None };
                                        session.get_mut().set_error(
                                            AmqpTransportError::SessionEnded(
                                                remote_end.error.clone(),
                                            ),
                                        );
                                        let id = session.get_mut().id();
                                        inner.post_frame(AmqpFrame::new(id, end.into()));
                                        if let Some(token) =
                                            inner.sessions_map.remove(&frame.channel_id())
                                        {
                                            inner.sessions.remove(token);
                                        }
                                    }
                                    _ => session.get_mut().handle_frame(frame.into_parts().1),
                                }
                            }
                            ChannelState::Closing(ref mut tx) => match frame.performative() {
                                Frame::End(frm) => {
                                    trace!("Session end is confirmed: {:?}", frm);
                                    if let Some(tx) = tx.take() {
                                        let _ = tx.send(Ok(()));
                                    }
                                    if let Some(token) =
                                        inner.sessions_map.remove(&frame.channel_id())
                                    {
                                        inner.sessions.remove(token);
                                    }
                                }
                                frm => trace!("Got frame after initiated session end: {:?}", frm),
                            },
                        }
                    } else {
                        error!("Can not find channel: {}", channel_id);
                        continue;
                    }
                }
                Poll::Ready(None) => {
                    trace!("Amqp connection has been disconnected with no error");
                    inner.set_error(AmqpTransportError::Disconnected);
                    return Poll::Ready(None);
                }
                Poll::Pending => {
                    self.hb.update_local(update);
                    break;
                }
                Poll::Ready(Some(Err(e))) => {
                    trace!("Error while reading from socket: {:?}", e);
                    inner.set_error(AmqpTransportError::Disconnected);
                    return Poll::Ready(Some(Err(e)));
                }
            }
        }

        Poll::Pending
    }
}

impl<T> Drop for Connection<T> {
    fn drop(&mut self) {
        trace!("Connection has been dropped, disconnecting");
        self.inner
            .get_mut()
            .set_error(AmqpTransportError::Disconnected);
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> Future for Connection<T> {
    type Output = Result<(), Either<AmqpCodecError, io::Error>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // connection heartbeat
        match self.hb.poll(cx) {
            HeartbeatAction::None => (),
            HeartbeatAction::Close => {
                trace!("Headerbeat expired");
                self.inner.get_mut().set_error(AmqpTransportError::Timeout);
                return Poll::Ready(Ok(()));
            }
            HeartbeatAction::Heartbeat => {
                trace!("Sending hb frame");
                self.inner
                    .get_mut()
                    .write_queue
                    .push_back(AmqpFrame::new(0, Frame::Empty));
            }
        }

        loop {
            match self.poll_incoming(cx) {
                Poll::Ready(None) => {
                    trace!("Connection is gone while reading");
                    self.inner
                        .get_mut()
                        .set_error(AmqpTransportError::Disconnected);
                    return Poll::Ready(Ok(()));
                }
                Poll::Ready(Some(Ok(frame))) => {
                    if let Some(channel) = self.inner.sessions.get(frame.channel_id() as usize) {
                        if let ChannelState::Established(ref session) = channel {
                            session.get_mut().handle_frame(frame.into_parts().1);
                            continue;
                        }
                    }
                    warn!("Unexpected frame: {:?}", frame);
                }
                Poll::Ready(Some(Err(e))) => {
                    trace!("Amqp connection read error: {:?}", e);
                    self.inner
                        .get_mut()
                        .set_error(AmqpTransportError::Disconnected);
                    return Poll::Ready(Err(e));
                }
                Poll::Pending => break,
            }
        }
        let _ = self.poll_outgoing(cx)?;
        self.register_write_task(cx);

        match self.poll_incoming(cx) {
            Poll::Ready(None) => {
                trace!("Connection is gone while reading 2");
                self.inner
                    .get_mut()
                    .set_error(AmqpTransportError::Disconnected);
                return Poll::Ready(Ok(()));
            }
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(channel) = self.inner.sessions.get(frame.channel_id() as usize) {
                    if let ChannelState::Established(ref session) = channel {
                        session.get_mut().handle_frame(frame.into_parts().1);
                        return Poll::Pending;
                    }
                }
                warn!("Unexpected frame: {:?}", frame);
            }
            Poll::Ready(Some(Err(e))) => {
                trace!("Amqp connection write error: {:?}", e);
                self.inner
                    .get_mut()
                    .set_error(AmqpTransportError::Disconnected);
                return Poll::Ready(Err(e));
            }
            Poll::Pending => (),
        }

        Poll::Pending
    }
}

#[derive(Clone)]
pub struct ConnectionController(pub(crate) Cell<ConnectionInner>);

impl ConnectionController {
    pub(crate) fn new(local: Configuration) -> ConnectionController {
        ConnectionController(Cell::new(ConnectionInner {
            local,
            remote: Configuration::default(),
            write_queue: VecDeque::new(),
            write_task: LocalWaker::new(),
            sessions: slab::Slab::with_capacity(8),
            sessions_map: AHashMap::default(),
            error: None,
            state: State::Normal,
        }))
    }

    pub(crate) fn set_remote(&mut self, remote: Configuration) {
        self.0.get_mut().remote = remote;
    }

    #[inline]
    /// Get remote connection configuration
    pub fn remote_config(&self) -> &Configuration {
        &self.0.get_ref().remote
    }

    #[inline]
    /// Drop connection
    pub fn drop_connection(&mut self) {
        let inner = self.0.get_mut();
        inner.state = State::Drop;
        inner.write_task.wake()
    }

    pub(crate) fn post_frame(&mut self, frame: AmqpFrame) {
        self.0.get_mut().post_frame(frame)
    }

    pub(crate) fn drop_session_copy(&mut self, _id: usize) {}
}

impl ConnectionInner {
    pub(crate) fn new(local: Configuration, remote: Configuration) -> ConnectionInner {
        ConnectionInner {
            local,
            remote,
            write_queue: VecDeque::new(),
            write_task: LocalWaker::new(),
            sessions: slab::Slab::with_capacity(8),
            sessions_map: AHashMap::default(),
            error: None,
            state: State::Normal,
        }
    }

    fn set_error(&mut self, err: AmqpTransportError) {
        log::trace!("Set connection error: {:?}", err);
        for (_, channel) in self.sessions.iter_mut() {
            match channel {
                ChannelState::Opening(_, _) | ChannelState::Closing(_) => (),
                ChannelState::Established(ref mut ses) => {
                    ses.get_mut().set_error(err.clone());
                }
            }
        }
        self.sessions.clear();
        self.sessions_map.clear();

        self.error = Some(err);
    }

    fn pop_next_frame(&mut self) -> Option<AmqpFrame> {
        self.write_queue.pop_front()
    }

    fn post_frame(&mut self, frame: AmqpFrame) {
        self.write_queue.push_back(frame);
        self.write_task.wake();
    }

    fn complete_session_creation(&mut self, channel_id: u16, begin: &Begin) {
        trace!(
            "Session opened: local {:?} remote {:?}",
            channel_id,
            begin.remote_channel()
        );

        let id = begin.remote_channel().unwrap() as usize;

        if let Some(channel) = self.sessions.get_mut(id) {
            if channel.is_opening() {
                if let ChannelState::Opening(tx, cell) = channel {
                    let cell = cell.upgrade().unwrap();
                    let session = Cell::new(SessionInner::new(
                        id,
                        true,
                        ConnectionController(cell),
                        channel_id,
                        begin.next_outgoing_id(),
                        begin.incoming_window(),
                        begin.outgoing_window(),
                    ));
                    self.sessions_map.insert(channel_id, id);

                    if tx
                        .take()
                        .unwrap()
                        .send(Session::new(session.clone()))
                        .is_err()
                    {
                        // todo: send end session
                    }
                    *channel = ChannelState::Established(session)
                }
            } else {
                // send error response
            }
        } else {
            // todo: rogue begin right now - do nothing. in future might indicate incoming attach
        }
    }
}
