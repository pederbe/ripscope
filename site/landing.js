// This head script prepares entrances before the body can paint. With JavaScript
// disabled, unsupported animation APIs, or reduced motion, the HTML stays visible.
(function () {
  if (window.matchMedia && Element.prototype.animate && 'IntersectionObserver' in window &&
      !window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
    document.documentElement.setAttribute('data-entrances-ready', '');
  }
  document.addEventListener('DOMContentLoaded', function () {

// Landing controls enhance content already present in the document.
(function () {
  var tabs = Array.from(document.querySelectorAll('.install-tabs [role="tab"]'));
  function select(tab, focus) {
    tabs.forEach(function (other) {
      var chosen = other === tab;
      other.setAttribute('aria-selected', String(chosen));
      other.tabIndex = chosen ? 0 : -1;
      document.getElementById(other.getAttribute('aria-controls')).hidden = !chosen;
    });
    if (focus) tab.focus();
  }
  tabs.forEach(function (tab, index) {
    tab.addEventListener('click', function () { select(tab, false); });
    tab.addEventListener('keydown', function (event) {
      var next;
      if (event.key === 'ArrowRight') next = tabs[(index + 1) % tabs.length];
      else if (event.key === 'ArrowLeft') next = tabs[(index + tabs.length - 1) % tabs.length];
      else if (event.key === 'Home') next = tabs[0];
      else if (event.key === 'End') next = tabs[tabs.length - 1];
      if (next) { event.preventDefault(); select(next, true); }
    });
  });

  document.querySelectorAll('button[data-copy]').forEach(function (button) {
    button.addEventListener('click', function () {
      var command = document.getElementById(button.dataset.copy);
      var status = document.getElementById('copy-status');
      function selectCommand() {
        var range = document.createRange();
        range.selectNodeContents(command);
        var selection = window.getSelection();
        selection.removeAllRanges();
        selection.addRange(range);
        status.textContent = 'Command selected. Press Ctrl+C or use your device’s copy action.';
      }
      if (!navigator.clipboard || !window.isSecureContext) { selectCommand(); return; }
      navigator.clipboard.writeText(command.textContent).then(function () {
        button.textContent = 'Copied';
        status.textContent = 'Copied to the clipboard';
        setTimeout(function () { button.textContent = 'Copy'; }, 1600);
      }, selectCommand);
    });
  });
})();

// Decorative motion runs only in view. All content remains usable without it.
(function () {
  var button = document.getElementById('motion-toggle');
  if (!button || !window.matchMedia || !Element.prototype.animate || !('IntersectionObserver' in window)) return;
  var reduced = window.matchMedia('(prefers-reduced-motion: reduce)');
  var requested = true;
  var effects = new Map();
  var entrances = new Map();
  var revealTargets = Array.from(document.querySelectorAll('[data-reveal]'));
  function pause(animation) {
    var time = animation.currentTime;
    animation.pause();
    if (time !== null) animation.currentTime = time;
  }
  function finishEntrance(element) {
    entranceObserver.unobserve(element);
    element.setAttribute('data-revealed', '');
    var animation = entrances.get(element);
    if (animation) animation.finish();
  }
  document.querySelectorAll('[data-motion], [data-aura]').forEach(function (element) {
    var kind = element.dataset.motion;
    var frames;
    var duration;
    if (kind === 'orbit' || kind === 'scan') {
      frames = [{ transform: 'rotate(0deg)' }, { transform: 'rotate(360deg)' }];
      duration = kind === 'orbit' ? 40000 : 18000;
    } else if (kind === 'breathe') {
      var opacity = Number(getComputedStyle(element).opacity);
      frames = [{ opacity: opacity * .6, transform: 'scale(.96)' },
        { opacity: opacity, transform: 'scale(1.05)' },
        { opacity: opacity * .6, transform: 'scale(.96)' }];
      duration = 7000;
    } else if (kind === 'flow') {
      frames = [{ transform: 'translateY(-10%)', opacity: 0 },
        { transform: 'translateY(0)', opacity: 1, offset: .15 },
        { transform: 'translateY(90%)', opacity: 1, offset: .85 },
        { transform: 'translateY(100%)', opacity: 0 }];
      duration = 4000;
    }
    var animation = frames && element.animate(frames, {
      duration: duration, iterations: Infinity, easing: kind === 'breathe' ? 'ease-in-out' : 'linear'
    });
    if (animation) animation.pause();
    effects.set(element, { animation: animation, visible: false });
  });
  function update() {
    var paused = !requested || reduced.matches;
    var changed = document.documentElement.hasAttribute('data-motion-paused') !== paused;
    document.documentElement.toggleAttribute('data-motion-paused', paused);
    effects.forEach(function (effect, element) {
      var running = !paused && !document.hidden && effect.visible;
      element.toggleAttribute('data-motion-active', running);
      if (effect.animation) {
        if (reduced.matches) effect.animation.cancel();
        else if (running) effect.animation.play();
        else pause(effect.animation);
      }
    });
    if (paused) revealTargets.forEach(finishEntrance);
    else entrances.forEach(function (animation) {
      if (document.hidden) pause(animation);
      else if (animation.playState === 'paused') animation.play();
    });
    button.disabled = reduced.matches;
    button.textContent = reduced.matches ? 'Motion off' : requested ? 'Pause motion' : 'Resume motion';
    button.setAttribute('aria-pressed', String(!requested));
    if (changed) document.dispatchEvent(new Event('ripscope-motion-change'));
  }
  var observer = new IntersectionObserver(function (entries) {
    entries.forEach(function (entry) {
      effects.get(entry.target).visible = entry.isIntersecting;
    });
    update();
  }, { threshold: .1 });
  var entranceObserver = new IntersectionObserver(function (entries) {
    var delay = 0;
    entries.forEach(function (entry) {
      if (!entry.isIntersecting || entry.intersectionRatio < .05) return;
      var element = entry.target;
      entranceObserver.unobserve(element);
      if (requested && !reduced.matches) {
        var animation = element.animate([
          { opacity: 0, transform: 'translateY(48px)' },
          { opacity: 1, transform: 'translateY(0)' }
        ], { duration: 1200, delay: delay, easing: 'cubic-bezier(.25,.1,.25,1)', fill: 'backwards' });
        entrances.set(element, animation);
        animation.onfinish = function () { entrances.delete(element); };
        delay = Math.min(delay + 140, 420);
      }
      element.setAttribute('data-revealed', '');
    });
    update();
  }, { rootMargin: '0px 0px -64px 0px', threshold: .05 });
  effects.forEach(function (_, element) { observer.observe(element); });
  revealTargets.forEach(function (element) { entranceObserver.observe(element); });
  document.addEventListener('focusin', function (event) {
    revealTargets.forEach(function (element) {
      if (!element.hasAttribute('data-revealed') && element.contains(event.target)) finishEntrance(element);
    });
  });
  button.addEventListener('click', function () { requested = !requested; update(); });
  reduced.addEventListener('change', update);
  document.addEventListener('visibilitychange', update);
  button.hidden = false;
  update();
})();

// The illustration's interface is one persistent user choice plus automatic pauses.
(function () {
  var demo = document.getElementById('editor-illustration');
  var visual = demo && demo.querySelector('.demo-visual');
  var button = document.getElementById('demo-toggle');
  var status = document.getElementById('demo-state');
  if (!demo || !visual || !button || !status || !window.matchMedia || !('IntersectionObserver' in window)) return;
  var reduced = window.matchMedia('(prefers-reduced-motion: reduce)');
  // One controller owns animation and pause state. The HTML is the static result.
  var keyframes = [
    [{ offset: 0, opacity: 1, transform: 'translateY(0)' },
      { offset: .27, opacity: 1, transform: 'translateY(0)' },
      { offset: .30, opacity: 0, transform: 'translateY(-3px)' },
      { offset: .97, opacity: 0, transform: 'translateY(-3px)' },
      { offset: 1, opacity: 1, transform: 'translateY(0)' }],
    [{ offset: 0, opacity: 0, transform: 'translateY(3px)' },
      { offset: .29, opacity: 0, transform: 'translateY(3px)' },
      { offset: .32, opacity: 1, transform: 'translateY(0)' },
      { offset: .61, opacity: 1, transform: 'translateY(0)' },
      { offset: .64, opacity: 0, transform: 'translateY(3px)' },
      { offset: 1, opacity: 0, transform: 'translateY(3px)' }],
    [{ offset: 0, opacity: 0, transform: 'translateY(3px)' },
      { offset: .63, opacity: 0, transform: 'translateY(3px)' },
      { offset: .66, opacity: 1, transform: 'translateY(0)' },
      { offset: .97, opacity: 1, transform: 'translateY(0)' },
      { offset: 1, opacity: 0, transform: 'translateY(3px)' }]
  ];
  var animations = Array.from(visual.querySelectorAll('.demo-scene')).map(function (scene, index) {
    var animation = scene.animate(keyframes[index], { duration: 18000, iterations: Infinity });
    animation.pause();
    return animation;
  });
  var requested = true;
  var visible = false;
  var hovered = false;
  var focused = false;
  function update() {
    var pagePaused = document.documentElement.hasAttribute('data-motion-paused');
    var reason = reduced.matches ? 'Reduced motion: final result' : !requested ? 'Paused' :
      pagePaused ? 'Page motion paused' :
      document.hidden ? 'Paused while page is hidden' : !visible ? 'Paused outside view' :
      hovered ? 'Paused on hover' : focused ? 'Paused on focus' : 'Playing · 18-second loop';
    var running = requested && !pagePaused && !reduced.matches && !document.hidden && visible && !hovered && !focused;
    animations.forEach(function (animation) {
      if (reduced.matches) animation.cancel();
      else if (running) animation.play();
      else {
        var time = animation.currentTime;
        animation.pause();
        if (time !== null) animation.currentTime = time;
      }
    });
    demo.toggleAttribute('data-demo-running', running);
    demo.toggleAttribute('data-demo-static', reduced.matches);
    button.textContent = requested ? 'Pause illustration' : 'Resume illustration';
    button.disabled = reduced.matches;
    if (reduced.matches) button.textContent = 'Motion disabled';
    status.textContent = reason;
  }
  button.addEventListener('click', function () { requested = !requested; update(); });
  visual.addEventListener('mouseenter', function () { hovered = true; update(); });
  visual.addEventListener('mouseleave', function () { hovered = false; update(); });
  visual.addEventListener('focusin', function () { focused = true; update(); });
  visual.addEventListener('focusout', function () { focused = false; update(); });
  document.addEventListener('visibilitychange', update);
  document.addEventListener('ripscope-motion-change', update);
  reduced.addEventListener('change', update);
  new IntersectionObserver(function (entries) {
    visible = entries[0].isIntersecting && entries[0].intersectionRatio >= .15;
    update();
  }, { threshold: .15 }).observe(visual);
  demo.setAttribute('data-demo-ready', '');
  update();
})();

  });
})();
