// Static tab stops keep wide code and tables keyboard-scrollable even without JavaScript.
export function focusScrollRegions(node) {
  if (node.type === 'element' && ['pre', 'table'].includes(node.tagName)) {
    node.properties ||= {};
    node.properties.tabIndex = 0;
  }
  for (const child of node.children || []) focusScrollRegions(child);
}

function wrapDiagrams(node) {
  if (!node.children) return;
  node.children = node.children.map(child => {
    if (child.type === 'element' && child.tagName === 'svg' && child.properties?.id?.startsWith('mermaid')) {
      return {
        type: 'element', tagName: 'div',
        properties: { className: ['v-diagram'], tabIndex: 0, role: 'region', ariaLabel: 'Scrollable engineering diagram' },
        children: [child],
      };
    }
    wrapDiagrams(child);
    return child;
  });
}
export function rehypeFocusScrollRegions() {
  return tree => {
    focusScrollRegions(tree);
    wrapDiagrams(tree);
  };
}
