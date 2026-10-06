---
title: Volna
description: Inspect a sample hardware simulation recording in the browser.
template: splash
hero:
  title: Explore a hardware trace
  tagline: Volna is a hardware debug environment for recordings of signals, transactions, and runtime events on one time base.
  actions:
    - text: Open Web App
      link: ./app/
      icon: right-arrow
      variant: primary
    - text: Read the Docs
      link: ./docs/
      variant: secondary
---

The sample below opens a VTR recording with a saved workspace showing waveforms,
pipeline activity, transactions, and a table. Select a panel, scroll to inspect
time, and use the viewer controls to move between events. Click inside the viewer
before using its keyboard shortcuts; scroll the surrounding page outside the viewer.

<div class="v-viewer-frame" role="region" aria-label="Volna sample viewer" tabindex="0">
  <iframe title="Interactive Volna sample recording" src="./viewer/" loading="eager" allow="clipboard-write"></iframe>
</div>
<div class="v-viewer-mobile-nav" aria-label="Viewer panels">
  <button class="v-btn v-btn--sm" type="button" data-viewer-scroll="left" aria-label="Show left viewer panels">← Left panels</button>
  <button class="v-btn v-btn--sm" type="button" data-viewer-scroll="right" aria-label="Show right viewer panels">Right panels →</button>
</div>
<script>
  const viewerFrame = document.querySelector('.v-viewer-frame');
  const updateViewerFrameTop = () => {
    if (!viewerFrame) return;
    // The intro can wrap at different widths, so measure where the frame starts.
    const top = viewerFrame.getBoundingClientRect().top + window.scrollY;
    viewerFrame.style.setProperty('--viewer-frame-top', `${top}px`);
  };
  updateViewerFrameTop();
  window.addEventListener('resize', updateViewerFrameTop);
  document.fonts?.ready.then(updateViewerFrameTop);
  document.querySelectorAll('[data-viewer-scroll]').forEach(button => {
    button.addEventListener('click', () => {
      document.querySelector('.v-viewer-frame')?.scrollBy({
        left: button.dataset.viewerScroll === 'right' ? 320 : -320,
      });
    });
  });
</script>

The viewer runs locally in your browser. If it cannot start, you can still
[open the Web App](./app/) or [read the engineering documentation](./docs/).
