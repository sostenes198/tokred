(function () {
  "use strict";
  var body = document.body;

  // Period toggle: switches between the precomputed 7 / 30 / all-days panes.
  var toolbar = document.getElementById("period-toolbar");
  if (toolbar) {
    toolbar.hidden = false;
    toolbar.addEventListener("click", function (event) {
      var button = event.target.closest("button[data-period]");
      if (!button) { return; }
      body.setAttribute("data-period", button.getAttribute("data-period"));
      toolbar.querySelectorAll("button").forEach(function (other) {
        other.setAttribute("aria-pressed", String(other === button));
      });
    });
  }

  // Tooltips: any element with a data-tip attribute (hover or keyboard focus).
  var tip = document.getElementById("tip");
  function place(x, y) {
    tip.style.left = Math.min(x + 12, window.innerWidth - tip.offsetWidth - 8) + "px";
    tip.style.top = Math.max(y - tip.offsetHeight - 12, 4) + "px";
  }
  function show(target, x, y) {
    tip.textContent = target.getAttribute("data-tip");
    tip.hidden = false;
    place(x, y);
  }
  document.addEventListener("mousemove", function (event) {
    var target = event.target.closest ? event.target.closest("[data-tip]") : null;
    if (target) { show(target, event.clientX, event.clientY); } else { tip.hidden = true; }
  });
  document.addEventListener("focusin", function (event) {
    var target = event.target.closest("[data-tip]");
    if (target) {
      var box = target.getBoundingClientRect();
      show(target, box.left + box.width / 2, box.top);
    }
  });
  document.addEventListener("focusout", function () { tip.hidden = true; });
  document.addEventListener("keydown", function (event) {
    if (event.key === "Escape") { tip.hidden = true; }
  });

  // The report data is embedded as JSON; expose it for ad-hoc inspection in the console.
  try {
    window.tokredReport = JSON.parse(document.getElementById("data").textContent);
  } catch (error) {
    window.tokredReport = null;
  }
})();
