/**
 * Jest mock for react-native.
 * Minimal mock for existing tests. Additional components can be unmocked
 * in test files via jest.mock('react-native', () => require('react-native'))
 * when full rendering is needed.
 */

// Create a simple View-like component factory
const createComponent = (displayName: string) => {
  const Component = (props: any) => {
    const React = require('react');
    return React.createElement(displayName, props, props.children);
  };
  Component.displayName = displayName;
  return Component;
};

export const Platform = { OS: 'android' };
export const Linking = { openURL: jest.fn().mockResolvedValue(undefined) };
export const Alert = { alert: jest.fn() };
export const StyleSheet = { 
  create: (styles: any) => styles,
  flatten: (style: any) => {
    if (Array.isArray(style)) return Object.assign({}, ...style);
    return style;
  },
};
export const View = createComponent('View');
export const Text = createComponent('Text');
export const ScrollView = createComponent('ScrollView');
export const KeyboardAvoidingView = createComponent('KeyboardAvoidingView');
export const TouchableOpacity = (props: any) => {
  const React = require('react');
  return React.createElement(
    'TouchableOpacity',
    { ...props, onPress: props.onPress },
    props.children
  );
};
export const TextInput = (props: any) => {
  const React = require('react');
  return React.createElement('TextInput', {
    ...props,
    onChangeText: (text: string) => props.onChangeText?.(text),
    onSubmitEditing: (e: any) => props.onSubmitEditing?.(e),
    onChange: props.onChange,
  }, props.children);
};
export const useColorScheme = () => 'light';
export const StatusBar = createComponent('StatusBar');
export const FlatList = require('react').forwardRef((props: any, ref: any) => {
  const React = require('react');
  // Expose imperative scroll methods so components calling
  // listRef.current?.scrollToEnd() in effects/timeouts don't throw in tests.
  React.useImperativeHandle(ref, () => ({
    scrollToEnd: () => {},
    scrollToOffset: () => {},
    scrollToIndex: () => {},
  }));
  return React.createElement('FlatList', { ...props, ref }, props.children);
});
export const ActivityIndicator = createComponent('ActivityIndicator');
export const SafeAreaView = createComponent('SafeAreaView');

// Minimal Animated mock — enough for components that use Animated.Value /
// Animated.View / timing / loop / sequence / parallel (e.g. InputBar waveform).
class AnimatedValue {
  _value: number;
  constructor(v: number) { this._value = v; }
  setValue(v: number) { this._value = v; }
  interpolate() { return this; }
}
const animation = { start: (cb?: (r: { finished: boolean }) => void) => cb?.({ finished: true }), stop: () => {} };
export const Animated = {
  Value: AnimatedValue,
  View: createComponent('Animated.View'),
  Text: createComponent('Animated.Text'),
  ScrollView: createComponent('Animated.ScrollView'),
  timing: () => animation,
  loop: () => animation,
  sequence: () => animation,
  parallel: () => animation,
  spring: () => animation,
};
export const Haptics = { impactAsync: jest.fn(), ImpactFeedbackStyle: { Light: 'light', Medium: 'medium' } };
