// On this workspace drive, Node's async realpath returns EPERM while sync realpath works.
// Apply only to this one local frontend build process.
const fs = require('node:fs');
const nativeRealpath = fs.realpathSync.native;
fs.realpathSync.native = (path, options) => {
  try {
    return nativeRealpath(path, options);
  } catch (error) {
    if (error?.code !== 'EPERM') throw error;
    return fs.realpathSync(path, options);
  }
};
const asyncRealpath = fs.promises.realpath.bind(fs.promises);
fs.promises.realpath = async (path, options) => {
  try {
    return await asyncRealpath(path, options);
  } catch (error) {
    if (error?.code !== 'EPERM') throw error;
    return fs.realpathSync(path, options);
  }
};
