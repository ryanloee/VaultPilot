// @ts-nocheck
/**
 * Issue #2943: InputBar TextInput lost onSubmitEditing/returnKeyType after
 * ChatScreen refactor — Enter-to-send regression (#2619).
 *
 * This test renders the REAL InputBar component (not a mock) and verifies the
 * underlying TextInput wires onSubmitEditing -> onSend and uses returnKeyType
 * "send". The pre-existing issue_2619_send_submit test mocks
 * '../../components/chat', so it never exercised the real InputBar TextInput.
 */
import React from 'react';
import { render, fireEvent, act } from '@testing-library/react-native';

// The shared react-native mock lacks Animated (InputBar uses Animated.Value),
// so provide a minimal self-contained mock here that exposes the pieces the
// component actually touches.
jest.mock('react-native', () => {
  const createComponent = (displayName: string) => {
    const C = (props: any) => React.createElement(displayName, props, props.children);
    (C as any).displayName = displayName;
    return C;
  };
  class AnimatedValue {
    public value: any;
    constructor(v: any) { this.value = v; }
    setValue(v: any) { this.value = v; }
    interpolate() { return new AnimatedValue(this.value); }
  }
  return {
    Platform: { OS: 'android' },
    StyleSheet: {
      create: (s: any) => s,
      flatten: (s: any) => (Array.isArray(s) ? Object.assign({}, ...s) : s),
    },
    View: createComponent('View'),
    Text: createComponent('Text'),
    ScrollView: createComponent('ScrollView'),
    TouchableOpacity: (props: any) =>
      React.createElement('TouchableOpacity', props, props.children),
    TextInput: (props: any) =>
      React.createElement('TextInput', {
        ...props,
        onChangeText: (t: string) => props.onChangeText?.(t),
        onSubmitEditing: (e: any) => props.onSubmitEditing?.(e),
      }, props.children),
    FlatList: (props: any) =>
      React.createElement('FlatList', props, props.children),
    Animated: {
      Value: AnimatedValue,
      parallel: () => ({ start: () => {}, stop: () => {} }),
      timing: () => ({ start: () => {}, stop: () => {} }),
    },
    useColorScheme: () => 'light',
  };
});

jest.mock('expo-haptics', () => ({
  impactAsync: jest.fn().mockResolvedValue(undefined),
  ImpactFeedbackStyle: { Medium: 'medium' },
}));

import InputBar from '../../components/chat/InputBar';

async function renderInputBar(overrides = {}) {
  const onSend = jest.fn();
  const baseProps = {
    input: '',
    inputHeight: 40,
    streaming: false,
    attachments: [],
    accentColor: '#007AFF',
    bgColor: '#FFF',
    inputBgColor: '#F9F9F9',
    textColor: '#000',
    textColorSecondary: '#666',
    borderColor: '#E0E0E0',
    voiceAvailable: false,
    voiceListening: false,
    voiceVolume: 0,
    onInputChange: jest.fn(),
    onInputHeightChange: jest.fn(),
    onSend,
    onStop: jest.fn(),
    onTakePhoto: jest.fn(),
    onPickImage: jest.fn(),
    onPickDocument: jest.fn(),
    onRemoveAttachment: jest.fn(),
    onVoiceToggle: jest.fn(),
    ...overrides,
  };
  const utils = await render(React.createElement(InputBar, baseProps));
  return { ...utils, onSend };
}

it('real InputBar TextInput has returnKeyType=send and onSubmitEditing wired', async () => {
  const { getByTestId } = await renderInputBar();
  const input = getByTestId('chat-input');
  // onSubmitEditing prop is present on the TextInput
  expect(typeof input.props.onSubmitEditing).toBe('function');
  // return key is configured for sending
  expect(input.props.returnKeyType).toBe('send');
});

it('pressing Enter (submitEditing) on InputBar triggers onSend', async () => {
  const { getByTestId, onSend } = await renderInputBar();
  const input = getByTestId('chat-input');
  await act(async () => {
    fireEvent(input, 'submitEditing');
  });
  expect(onSend).toHaveBeenCalledTimes(1);
});
