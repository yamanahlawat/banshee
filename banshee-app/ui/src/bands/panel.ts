/// The key a panel publishes its focus under. A band that destroys the control
/// the keyboard was on asks for this rather than naming a node in the panel's
/// own markup.
export const PANEL = Symbol('panel');

export type PanelFocus = { refocus: () => void };
