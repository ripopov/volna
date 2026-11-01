export function copyText(text, host = globalThis) {
  let write;
  try {
    if (!host.navigator?.clipboard?.writeText) throw new Error("Clipboard unavailable");
    write = host.navigator.clipboard.writeText(text);
  } catch (error) {
    write = Promise.reject(error);
  }
  Promise.resolve(write).catch(() => showFallback(text, host));
}

function showFallback(value, host) {
  const document = host.document;
  if (!document?.body) return;
  const prior = document.querySelector?.("dialog[data-volna-copy]");
  prior?.remove();

  const dialog = document.createElement("dialog");
  dialog.dataset.volnaCopy = "";
  dialog.setAttribute("aria-label", "Copy text");
  dialog.style.cssText = "margin:auto;padding:20px;max-width:90vw;width:680px;background:Canvas;color:CanvasText;border:1px solid GrayText;border-radius:6px;font:14px system-ui";
  const title = document.createElement("h2");
  title.textContent = "Copy text";
  const status = document.createElement("p");
  status.setAttribute("role", "status");
  status.textContent = "The host blocked copying. Retry, or select the complete text below and copy it.";
  const area = document.createElement("textarea");
  area.readOnly = true;
  area.value = value;
  area.setAttribute("aria-label", "Text to copy");
  area.style.cssText = "display:block;box-sizing:border-box;width:100%;height:220px;margin:12px 0;white-space:pre;font:13px monospace;user-select:text;-webkit-user-select:text";
  const retry = document.createElement("button");
  retry.textContent = "Retry copy";
  const close = document.createElement("button");
  close.textContent = "Close";
  const previousFocus = document.activeElement;
  const dispose = () => {
    area.value = "";
    dialog.close?.();
    dialog.remove();
    if (previousFocus?.isConnected) previousFocus.focus();
  };
  retry.addEventListener("click", () => {
    retry.disabled = true;
    status.textContent = "Copying…";
    let result;
    try {
      result = host.navigator.clipboard.writeText(value);
    } catch (error) {
      result = Promise.reject(error);
    }
    Promise.resolve(result).then(dispose, () => {
      retry.disabled = false;
      status.textContent = "Copying is still blocked. Select the text and use the host Copy command.";
      area.focus();
      area.select();
    });
  });
  close.addEventListener("click", dispose);
  dialog.addEventListener("cancel", event => {
    event.preventDefault();
    dispose();
  });
  for (const name of ["keydown", "keyup", "copy"]) {
    dialog.addEventListener(name, event => event.stopPropagation());
  }
  dialog.append(title, status, area, retry, close);
  document.body.append(dialog);
  dialog.showModal?.();
  area.focus();
  area.select();
}
