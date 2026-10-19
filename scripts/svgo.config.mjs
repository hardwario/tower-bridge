// svgo for the HARDWARIO TOWER logo. Multipass, but without convertToQ: its
// cubic-to-quadratic conversion bent the outlines by up to 0.08 viewBox
// units (the O of TOWER); without it the outlines stay within 0.007 units.
export default {
  multipass: true,
  plugins: [
    {
      name: "preset-default",
      params: {
        overrides: {
          convertPathData: { convertToQ: false },
          // Inline every rule, so the <style> element, the classes and ids go.
          inlineStyles: { onlyMatchedOnce: false },
          // Illustrator's data-name attributes.
          removeUnknownsAndDefaults: { keepDataAttrs: false },
        },
      },
    },
    // style="fill:..." to fill="...".
    "convertStyleToAttrs",
  ],
};
