# Mobile Regression Tests

Place regression tests here for mobile-specific bugs.

## Naming Convention

```
issue_NNN_short_description.test.ts
```

## Template

```typescript
/**
 * Regression test for issue #NNN: <title>
 *
 * Bug: <description>
 * Root cause: <cause>
 * Fix: PR #NNN / commit abc1234
 */

describe('Regression: Issue #NNN', () => {
  test('should <expected behavior>', () => {
    // Arrange
    // ...
    // Act
    // ...
    // Assert
    // expect(result).toBe(expected);
  });
});
```

## Running

```bash
cd mobile
npx jest --testPathPattern=regression
```
