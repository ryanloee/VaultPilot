/**
 * Unit tests for aiActions utility module.
 *
 * These tests validate the prompt building, action metadata, and
 * validation logic without making actual API calls.
 */

import {
  listAiActions,
  getAiActionInfo,
  executeAiAction,
  AiActionRequest,
} from '../utils/aiActions';

// Mock the chat API to avoid actual network calls
jest.mock('../api/client', () => ({
  chat: jest.fn(),
  parseSSEStream: jest.fn(),
}));

describe('listAiActions', () => {
  it('returns all 8 actions', () => {
    const actions = listAiActions();
    expect(actions).toHaveLength(8);
  });

  it('includes cleanUp action (#2685)', () => {
    const actions = listAiActions();
    const cleanUp = actions.find(a => a.id === 'cleanUp');
    expect(cleanUp).toBeDefined();
    expect(cleanUp!.label).toBe('整理');
    expect(cleanUp!.icon).toBeTruthy();
    expect(cleanUp!.description).toBeTruthy();
  });

  it('each action has required fields', () => {
    for (const action of listAiActions()) {
      expect(action.id).toBeTruthy();
      expect(action.label).toBeTruthy();
      expect(action.icon).toBeTruthy();
      expect(action.description).toBeTruthy();
    }
  });

  it('returns immutable copies', () => {
    const a = listAiActions();
    const b = listAiActions();
    expect(a).toEqual(b);
    // Mutation should not affect original
    (a[0] as any).id = 'hacked';
    expect(b[0].id).not.toBe('hacked');
  });
});

describe('getAiActionInfo', () => {
  it('finds known actions by id', () => {
    expect(getAiActionInfo('summarize')?.label).toBe('总结');
    expect(getAiActionInfo('translate')?.label).toBe('翻译');
    expect(getAiActionInfo('extractTodos')?.label).toBe('提取待办');
    expect(getAiActionInfo('cleanUp')?.label).toBe('整理');
  });

  it('returns undefined for unknown id', () => {
    expect(getAiActionInfo('unknown_action')).toBeUndefined();
  });

  it('is case sensitive', () => {
    expect(getAiActionInfo('Summarize')).toBeUndefined();
  });
});

describe('executeAiAction - validation', () => {
  it('returns error for empty text on summarize', async () => {
    const result = await executeAiAction({
      action: 'summarize',
      text: '',
    });
    expect(result.error).toBeTruthy();
    expect(result.isSuccess).toBe(false);
  });

  it('returns error for whitespace-only text', async () => {
    const result = await executeAiAction({
      action: 'summarize',
      text: '   ',
    });
    expect(result.error).toBeTruthy();
  });

  it('allows empty text for findRelatedNotes', async () => {
    const result = await executeAiAction({
      action: 'findRelatedNotes',
      text: '',
    });
    // Should not be a validation error — the mock chat will "fail" differently
    expect(result.error).toBeUndefined();
  });

  it('valid text passes validation (will fail on API call)', async () => {
    const result = await executeAiAction({
      action: 'summarize',
      text: 'Some real content to summarize.',
    });
    // No validation error expected — but chat mock returns nothing valid
    expect(result.error).toBeUndefined();
    // Since we mocked chat to do nothing, result should be empty
    expect(result.result).toBe('');
  });
});

describe('AiActionRequest - action field required', () => {
  it('each action type can be used in a request', () => {
    const actions = listAiActions();
    for (const a of actions) {
      const request: AiActionRequest = {
        action: a.id,
        text: 'Test content',
      };
      expect(request.action).toBe(a.id);
      expect(request.text).toBe('Test content');
    }
  });
});
