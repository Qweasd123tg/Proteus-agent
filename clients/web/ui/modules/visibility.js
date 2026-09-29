function ancestors(root) {
  const nodes = [];
  for (let node = root; node; node = node.parentNode ?? node.host)
    nodes.push(node);
  return nodes;
}

export function logicallyVisible(root) {
  return root.isConnected && !ancestors(root).some((node) => node.hidden);
}

// Observe explicit host visibility, including across ShadowRoot boundaries.
// Direct-child changes let a retained root move without watching all page DOM.
export function watchLogicalVisibility(root, changed) {
  let watched = [],
    visible;
  const observer = new MutationObserver(update);
  function update() {
    const chain = ancestors(root);
    if (
      chain.length !== watched.length ||
      chain.some((node, i) => node !== watched[i])
    ) {
      observer.disconnect();
      watched = chain;
      for (const node of chain)
        observer.observe(node, {
          childList: true,
          ...(node.nodeType === 1
            ? { attributes: true, attributeFilter: ["hidden"] }
            : {}),
        });
    }
    const next = root.isConnected && !chain.some((node) => node.hidden);
    if (next !== visible) {
      visible = next;
      changed(next);
    }
  }
  update();
  return () => observer.disconnect();
}
