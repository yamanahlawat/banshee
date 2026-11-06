/// The key a page publishes its focus under. A part that destroys the control
/// the keyboard was on asks for this rather than naming a node in the page's
/// own markup.
export const PAGE = Symbol('page');

export type PageFocus = { refocus: () => void };

/// The one heading a page lands focus on, whichever page is showing.
export const PAGE_HEADING = 'page-heading';

/// The sidebar's navigation, wherever it is drawn.
export const SIDEBAR = 'sidebar';
