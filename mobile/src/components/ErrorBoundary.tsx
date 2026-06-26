import React from 'react';
import { View, Text, TouchableOpacity, StyleSheet, Appearance } from 'react-native';

interface Props {
  children: React.ReactNode;
}

interface State {
  hasError: boolean;
  error: Error | null;
}

export default class ErrorBoundary extends React.Component<Props, State> {
  state: State = { hasError: false, error: null };

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error('[ErrorBoundary]', error, info.componentStack);
  }

  handleRetry = () => {
    this.setState({ hasError: false, error: null });
  };

  render() {
    if (this.state.hasError) {
      const isDark = Appearance.getColorScheme() === 'dark';
      return (
        <View style={[styles.container, { backgroundColor: isDark ? '#1a1a1a' : '#fff' }]}>
          <Text style={[styles.title, { color: isDark ? '#e0e0e0' : '#1a1a1a' }]}>应用出错了</Text>
          <Text style={[styles.message, { color: isDark ? '#aaa' : '#666' }]}>{this.state.error?.message}</Text>
          <TouchableOpacity style={[styles.button, { backgroundColor: isDark ? '#4a8ac7' : '#1E3A5F' }]} onPress={this.handleRetry} accessibilityRole="button" accessibilityLabel="重试">
            <Text style={styles.buttonText}>重试</Text>
          </TouchableOpacity>
        </View>
      );
    }
    return this.props.children;
  }
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    justifyContent: 'center',
    alignItems: 'center',
    padding: 24,
  },
  title: {
    fontSize: 20,
    fontWeight: 'bold',
    marginBottom: 12,
  },
  message: {
    fontSize: 14,
    textAlign: 'center',
    marginBottom: 24,
  },
  button: {
    paddingHorizontal: 24,
    paddingVertical: 12,
    borderRadius: 8,
  },
  buttonText: {
    color: '#fff',
    fontSize: 16,
    fontWeight: '600',
  },
});
