// Mock for @expo/vector-icons/Ionicons
const React = require('react');

function Ionicons(props) {
  return React.createElement('Ionicons', props);
}
Ionicons.glyphMap = new Proxy({}, { get: () => 'mock-glyph' });

module.exports = Ionicons;
module.exports.default = Ionicons;
