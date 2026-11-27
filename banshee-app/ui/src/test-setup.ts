// jsdom has no Web Animations, and Svelte plays transitions through them. Each
// animation here finishes the moment it is told what to do when it finishes.
Element.prototype.animate = function () {
  return {
    cancel() {},
    currentTime: 0,
    playState: 'finished',
    effect: null,
    finished: Promise.resolve(),
    set onfinish(finish: (() => void) | null) {
      finish?.();
    },
  } as unknown as Animation;
};
