//! Text 렌더링 캐시 키/항목 정의 — 책임 단위로 나눈 하위 모듈 모음.

mod effective_style;
mod hit_test;
mod layout;
mod measure;
mod outline_bitmap;
mod tracker;

pub(super) use effective_style::EffectiveOutlineStyle;
pub(super) use hit_test::{HitTestCache, HitTestKeyRef};
pub(super) use layout::{LayoutKeyRef, TextLayoutCache};
pub(super) use measure::{MeasureCache, MeasureKeyRef};
pub(super) use outline_bitmap::{OutlineBitmap, OutlineBitmapKey, OutlineBitmapKeyRef};
pub(super) use tracker::{MissTracker, UnusedSlotTracker};
