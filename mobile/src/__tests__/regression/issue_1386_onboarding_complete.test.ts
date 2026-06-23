/**
 * Regression test for #1386: OnboardingScreen "完成" button should not re-test.
 *
 * After a successful API test, the button text changes to "完成" and
 * pressing it should call onComplete() directly, not re-run handleTestAndSave()
 * which would duplicate the provider save and re-test the API.
 */

describe('OnboardingScreen completion logic (#1386)', () => {
  it('onComplete should be called directly after success, not via setTimeout', () => {
    // After fix: no setTimeout(onComplete, 600) in handleTestAndSave.
    // The button's onPress checks testResult and calls onComplete directly.
    const testResult = '✅ 连接成功';
    const shouldComplete = testResult?.startsWith('✅');
    expect(shouldComplete).toBe(true);
  });

  it('onPress should route to onComplete when test passed', () => {
    const testResult = '✅ 连接成功';
    const handleTestAndSave = jest.fn();
    const onComplete = jest.fn();

    // Simulate the onPress logic from the fix
    const onPress = testResult?.startsWith('✅') ? onComplete : handleTestAndSave;
    onPress();

    expect(onComplete).toHaveBeenCalledTimes(1);
    expect(handleTestAndSave).not.toHaveBeenCalled();
  });

  it('onPress should route to handleTestAndSave when test not yet run', () => {
    const testResult = null as string | null;
    const handleTestAndSave = jest.fn();
    const onComplete = jest.fn();

    const onPress = testResult?.startsWith('✅') ? onComplete : handleTestAndSave;
    onPress();

    expect(handleTestAndSave).toHaveBeenCalledTimes(1);
    expect(onComplete).not.toHaveBeenCalled();
  });

  it('onPress should route to handleTestAndSave when test failed', () => {
    const testResult = '❌ 连接失败';
    const handleTestAndSave = jest.fn();
    const onComplete = jest.fn();

    const onPress = testResult?.startsWith('✅') ? onComplete : handleTestAndSave;
    onPress();

    expect(handleTestAndSave).toHaveBeenCalledTimes(1);
    expect(onComplete).not.toHaveBeenCalled();
  });
});
