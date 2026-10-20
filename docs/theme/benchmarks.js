// All tables are readable without JavaScript. Selectors show one view at a time.
(function () {
  var controls = document.getElementById('benchmark-controls');
  if (!controls) return;
  var frame = document.getElementById('benchmark-frame');
  var project = document.getElementById('benchmark-project');
  var mode = document.getElementById('benchmark-mode');
  function update() {
    document.querySelectorAll('.benchmark-panel').forEach(function (panel) {
      panel.hidden = panel.dataset.frame !== frame.value ||
        panel.dataset.project !== project.value || panel.dataset.mode !== mode.value;
      panel.open = !panel.hidden;
    });
  }
  controls.addEventListener('change', update);
  update();
  controls.hidden = false;
})();
