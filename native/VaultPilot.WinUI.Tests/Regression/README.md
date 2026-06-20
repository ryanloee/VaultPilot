# WinUI Regression Tests

Place regression tests here for WinUI-specific bugs.

## Naming Convention

```
IssueNNNShortDescriptionTests.cs
```

## Template

```csharp
using Xunit;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #NNN: title
/// Bug: description
/// Root cause: cause
/// Fix: PR #NNN / commit abc1234
/// </summary>
public class IssueNNNTests
{
    [Fact]
    public void Regression_NNN_ShouldExpectedBehavior()
    {
        // Arrange
        // ...
        // Act
        // ...
        // Assert
        // Assert.Equal(expected, actual);
    }
}
```

## Running

```bash
dotnet test native/VaultPilot.WinUI.Tests/ --filter "Regression"
```
