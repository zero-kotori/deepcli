use ratatui::layout::Rect;

pub(super) fn rect_contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && column < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}

pub(super) fn rect_content_row_contains(area: Rect, row: u16) -> bool {
    row > area.y && row < area.y.saturating_add(area.height).saturating_sub(1)
}
