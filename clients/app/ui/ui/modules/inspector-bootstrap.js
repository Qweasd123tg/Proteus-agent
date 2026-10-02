// Runs before WASM. An embedded Inspector uses the same native connection.
if (
  window.parent !== window &&
  new URL(location.href).searchParams.get("embedded") === "true"
) {
  document.documentElement.classList.add("embedded-inspector");
  try {
    if (!window.__PROTEUS_DESKTOP__ && parent.__PROTEUS_DESKTOP__) {
      Object.defineProperty(window, "__PROTEUS_DESKTOP__", {
        value: Object.freeze({ ...parent.__PROTEUS_DESKTOP__ }),
      });
    }
  } catch {
    /* Browser clients can use separate origins; their URL carries bootstrap. */
  }
  document.addEventListener(
    "click",
    (event) => {
      const link = event.target.closest?.("a");
      if (
        !link ||
        (!link.matches(".analysis-open-chat,.inspector-chat-link") &&
          !link.href.startsWith("proteus-desktop:chat"))
      )
        return;
      event.preventDefault();
      event.stopImmediatePropagation();
      parent.postMessage(
        { type: "proteus-open-chat", href: link.href },
        new URL(document.referrer || location.href).origin,
      );
    },
    true,
  );
}

// Native/same-origin embedding keeps application shortcuts when the frame has focus.
if(window.parent!==window){
  window.addEventListener('keydown',event=>{
    if(event.defaultPrevented)return;
    try{
      const forwarded=new parent.KeyboardEvent('keydown',{key:event.key,code:event.code,ctrlKey:event.ctrlKey,metaKey:event.metaKey,shiftKey:event.shiftKey,altKey:event.altKey,repeat:event.repeat,isComposing:event.isComposing,bubbles:true,cancelable:true});
      if(!parent.document.dispatchEvent(forwarded))event.preventDefault();
    }catch{/* Separate browser origins own independent shortcut dispatch. */}
  });
}
