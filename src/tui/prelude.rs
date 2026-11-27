//! Shared imports for the unchanged screen and modal bodies.
pub(super) use std::cell::{Cell, RefCell};
pub(super) use std::collections::{HashMap, HashSet};
pub(super) use std::sync::mpsc::Sender;
pub(super) use std::sync::Arc;
pub(super) use std::time::Duration;

pub(super) use anyhow::Result;
pub(super) use ftui::layout::{Constraint, Flex, Rect, Sides};
pub(super) use ftui::runtime::{Every, StopSignal, SubId, Subscription};
pub(super) use ftui::text::wrap::WrapMode;
pub(super) use ftui::text::{Line, Span, Text};
pub(super) use ftui::widgets::block::Block;
pub(super) use ftui::widgets::borders::BorderType;
pub(super) use ftui::widgets::input::TextInput;
pub(super) use ftui::widgets::list::ListState;
pub(super) use ftui::widgets::paragraph::Paragraph;
pub(super) use ftui::widgets::{StatefulWidget, StatusLine, Widget};
pub(super) use ftui::{Cmd, Event, Frame, KeyCode, KeyEvent, KeyEventKind, Model, Modifiers, Style};

pub(super) use crate::config::Config;
pub(super) use crate::imdb::{self, Title};
pub(super) use crate::browse;
pub(super) use crate::shows::{Episode, Show, Shows};
pub(super) use crate::qbt::{human_bytes, human_eta, human_rate, ymd, AddOptions, RssFeed, SearchPlugin, SearchResult, TorrentFile, TransferInfo};
pub(super) use crate::api::Torrent;
pub(super) use super::remote::ScreenClient as Client;
pub(super) use super::poster::{KittyPlacer, Poster};
pub(super) use super::theme::Theme;
