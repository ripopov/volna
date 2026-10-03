function initializeTimeUnits() {
  for (const widget of document.querySelectorAll<HTMLElement>('[data-time-unit]')) {
    const ticks = widget.querySelector<HTMLInputElement>('[data-ticks]')!;
    const scale = widget.querySelector<HTMLSelectElement>('[data-scale]')!;
    const output = widget.querySelector<HTMLOutputElement>('output')!;
    const update = () => {
      // Parse as BigInt: trace timestamps can exceed JavaScript's exact Number range.
      if (!/^\d+$/.test(ticks.value) || BigInt(ticks.value) > 18446744073709551615n) {
        output.textContent = 'Enter an integer from 0 to 18446744073709551615.';
        ticks.setAttribute('aria-invalid', 'true');
        return;
      }
      ticks.removeAttribute('aria-invalid');
      const value = BigInt(ticks.value);
      const places = -Number(scale.value);
      const digits = value.toString().padStart(places + 1, '0');
      const seconds = places ? `${digits.slice(0, -places)}.${digits.slice(-places)}` : digits;
      output.textContent = `${seconds} s`;
    };
    ticks.addEventListener('input', update);
    scale.addEventListener('change', update);
    update();
  }
}
initializeTimeUnits();
