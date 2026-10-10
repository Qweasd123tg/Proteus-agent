// Popover opacity follows this registered number instead of transitioning
// opacity itself; motion and its exit snapshot both animate it.
export const alphaProperty = '--popover-alpha';
try {
  CSS.registerProperty({ name: alphaProperty, syntax: '<number>', inherits: false, initialValue: '1' });
} catch (error) {
  // A second copy of this module (another URL or realm) already registered it.
  if (error?.name !== 'InvalidModificationError') throw error;
}
