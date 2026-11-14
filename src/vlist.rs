//! A virtualized list: only the rows on screen are built, so lists of tens of thousands of
//! local charts stay instant to draw and scroll.

use iced::widget::{column, container, scrollable, space};
use iced::{Element, Length, Task};

pub const ROW_H: f32 = 34.0;

#[derive(Debug, Default, Clone, Copy)]
pub struct VList {
    offset: f32,
    height: f32,
    /// Lets the list be scrolled back to the top after its contents change.
    id: Option<&'static str>,
    pending_top: bool,
}

impl VList {
    pub fn on_scroll(&mut self, vp: scrollable::Viewport) {
        self.offset = vp.absolute_offset().y;
        self.height = vp.bounds().height;
    }

    pub fn set_id(&mut self, id: &'static str) {
        self.id = Some(id);
    }

    /// The contents changed: scroll back to the top (see `take_scroll_task`).
    pub fn reset(&mut self) {
        self.offset = 0.0;
        self.pending_top = true;
    }

    /// Task that scrolls the list to the top if `reset` was called since the last call.
    pub fn take_scroll_task<T: Send + 'static>(&mut self) -> Task<T> {
        match (std::mem::take(&mut self.pending_top), self.id) {
            (true, Some(id)) => iced::widget::operation::snap_to(id, scrollable::RelativeOffset::START),
            _ => Task::none(),
        }
    }

    /// Rows `first..last` to build for a list of `len` rows, where row `open` (if any) has
    /// `extra` pixels of details below it.
    fn window(&self, len: usize, open: Option<(usize, f32)>) -> (usize, usize) {
        let height = if self.height > 0.0 { self.height } else { 1200.0 };
        // Rows below the open one sit `extra` lower.
        let offset = match open {
            Some((o, extra)) if self.offset > (o + 1) as f32 * ROW_H => (self.offset - extra).max((o + 1) as f32 * ROW_H),
            _ => self.offset,
        };
        let first = ((offset / ROW_H) as usize).saturating_sub(2).min(len);
        let last = (first + (height / ROW_H) as usize + 5).min(len);
        (first, last)
    }

    /// The list, with `open` = (row, details height, details) shown below that row.
    pub fn view<'a, M: 'a>(
        &self,
        len: usize,
        row: impl Fn(usize) -> Element<'a, M>,
        open: Option<(usize, f32, Element<'a, M>)>,
        on_scroll: impl Fn(scrollable::Viewport) -> M + 'a,
    ) -> Element<'a, M> {
        let (first, last) = self.window(len, open.as_ref().map(|(i, h, _)| (*i, *h)));
        let (open_at, extra, mut details) = match open {
            Some((i, h, e)) if i < len => (Some(i), h, Some(e)),
            _ => (None, 0.0, None),
        };
        // The details take their space even when their row is scrolled out of the built window.
        let above = open_at.filter(|&o| o < first).map_or(0.0, |_| extra);
        let below = open_at.filter(|&o| o >= last).map_or(0.0, |_| extra);
        let mut col = column![space().height(first as f32 * ROW_H + above)];
        for i in first..last {
            col = col.push(container(row(i)).height(ROW_H).width(Length::Fill));
            if open_at == Some(i)
                && let Some(d) = details.take()
            {
                col = col.push(container(d).height(extra).width(Length::Fill));
            }
        }
        col = col.push(space().height((len - last) as f32 * ROW_H + below));
        let mut s = scrollable(col.width(Length::Fill)).on_scroll(on_scroll).height(Length::Fill).width(Length::Fill);
        if let Some(id) = self.id {
            s = s.id(id);
        }
        s.into()
    }
}
