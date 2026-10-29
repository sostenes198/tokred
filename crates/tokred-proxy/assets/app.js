// Placeholder: the real app replaces this file. It only shows that the API answers.
fetch('/dashboard/api/sessions')
  .then((response) => response.json())
  .then((data) => {
    document.getElementById('status').textContent =
      data.active.length + ' active, ' + data.recent.length + ' recent sessions';
  });
