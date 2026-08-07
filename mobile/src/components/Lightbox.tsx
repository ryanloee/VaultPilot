/**
 * Image Lightbox component (#3030, #3422, #3790).
 *
 * Fullscreen image viewer with:
 * - Semi-transparent dark overlay (#3030)
 * - Close button + tap backdrop to close (#3030)
 * - Prev/next navigation between multiple images (#3030)
 * - Thumbnail position indicator (#3030)
 * - Swipe-down to dismiss gesture (#3422)
 * - Double-tap to toggle zoom (#3422)
 * - Pinch / button zoom with pan support (#3422, #3790)
 * - Zoom percentage indicator (#3422)
 * - Fit-to-screen zoom control (#3790)
 *
 * Uses only React Native core APIs (Animated, PanResponder, Modal, Image) —
 * no external gesture-handler dependencies required.
 */
import React, { useCallback, useMemo, useRef, useState } from 'react';
import {
  Animated,
  Dimensions,
  Image as RNImage,
  Modal,
  PanResponder,
  Platform,
  StyleSheet,
  Text,
  TouchableOpacity,
  View,
} from 'react-native';
import Icon from './Icon';
import {
  clampZoom,
  imageFileName,
  MarkdownImage,
  nextImageIndex,
  nextZoomOnDoubleTap,
  shouldDismissOnSwipe,
  SWIPE_DISMISS_THRESHOLD,
  zoomPercentage,
  MIN_ZOOM,
} from '../utils/imageMarkdown';

export interface LightboxProps {
  /** Whether the modal is visible */
  visible: boolean;
  /** Array of images to display */
  images: MarkdownImage[];
  /** Index of the currently displayed image */
  index: number;
  /** Called when the user requests to close (backdrop tap, ✕, swipe-down, Esc) */
  onClose: () => void;
  /** Called when the displayed index changes (navigation) */
  onIndexChange?: (index: number) => void;
}

const SCREEN_WIDTH = Dimensions.get('window').width;
const SCREEN_HEIGHT = Dimensions.get('window').height;

export default function Lightbox({
  visible,
  images,
  index,
  onClose,
  onIndexChange,
}: LightboxProps) {
  // #3945: ALL hooks (useState/useRef/useCallback/useMemo) MUST be called
  // unconditionally before any early return. Moving the empty-images guard
  // below the hooks would violate the React Rules of Hooks and crash with
  // "Rendered more/fewer hooks than expected" when the images array transitions
  // between empty and non-empty across renders. We compute safeIndex/current
  // in an empty-safe way so the hooks have stable inputs.

  const safeIndex = images.length ? Math.min(index, images.length - 1) : 0;

  // ── Zoom state ──
  const [zoom, setZoom] = useState(1);
  // Pan offset for zoomed image (animated)
  const panX = useRef(new Animated.Value(0)).current;
  const panY = useRef(new Animated.Value(0)).current;
  // Vertical drag for swipe-down dismiss (animated)
  const dismissY = useRef(new Animated.Value(0)).current;
  // Last tap timestamp for double-tap detection
  const lastTapRef = useRef(0);

  const resetTransforms = useCallback(() => {
    panX.setValue(0);
    panY.setValue(0);
    dismissY.setValue(0);
  }, [panX, panY, dismissY]);

  // ── Navigation ──
  const navigate = useCallback(
    (delta: number) => {
      if (!images.length) return;
      const next = nextImageIndex(safeIndex, delta, images.length);
      if (next !== safeIndex) {
        setZoom(1);
        resetTransforms();
        onIndexChange?.(next);
      }
    },
    [safeIndex, images.length, onIndexChange, resetTransforms],
  );

  // ── Zoom handlers ──
  const zoomIn = useCallback(() => {
    setZoom((z) => clampZoom(z + 0.5));
  }, []);

  const zoomOut = useCallback(() => {
    setZoom((z) => {
      const next = clampZoom(z - 0.5);
      if (next <= MIN_ZOOM) {
        resetTransforms();
      }
      return next;
    });
  }, [resetTransforms]);

  const fitToScreen = useCallback(() => {
    setZoom(MIN_ZOOM);
    resetTransforms();
  }, [resetTransforms]);

  const handleDoubleTap = useCallback(() => {
    setZoom((z) => {
      const next = nextZoomOnDoubleTap(z);
      if (next === MIN_ZOOM) {
        resetTransforms();
      }
      return next;
    });
  }, [resetTransforms]);

  // ── PanResponder for swipe-down dismiss + pan when zoomed ──
  const panResponder = useMemo(() => {
    let startDismissY = 0;
    let startPanX = 0;
    let startPanY = 0;

    return PanResponder.create({
      onMoveShouldSetPanResponder: (_evt, gestureState) => {
        // Only capture when there's actual movement
        return Math.abs(gestureState.dx) > 5 || Math.abs(gestureState.dy) > 5;
      },

      onPanResponderGrant: (_evt, _gestureState) => {
        // #3455: Capture the current animated offsets so each gesture
        // continues from where the last one left off. Resetting to 0
        // discards accumulated pan, making the zoomed image snap to
        // center at the start of every new pan gesture.
        startDismissY = (dismissY as any)._value || 0;
        startPanX = (panX as any)._value || 0;
        startPanY = (panY as any)._value || 0;
      },

      onPanResponderMove: (_evt, gestureState) => {
        if (zoom > MIN_ZOOM) {
          // When zoomed: pan the image
          panX.setValue(startPanX + gestureState.dx);
          panY.setValue(startPanY + gestureState.dy);
        } else {
          // When not zoomed: track vertical drag for swipe-down dismiss
          // Only allow downward drag
          const drag = Math.max(0, gestureState.dy);
          dismissY.setValue(startDismissY + drag);
        }
      },

      onPanResponderRelease: (_evt, gestureState) => {
        if (zoom <= MIN_ZOOM) {
          // Check swipe-down dismiss
          if (shouldDismissOnSwipe(gestureState.dy)) {
            // Animate out then close
            Animated.timing(dismissY, {
              toValue: SCREEN_HEIGHT,
              duration: 200,
              useNativeDriver: true,
            }).start(() => {
              resetTransforms();
              onClose();
            });
          } else {
            // Snap back
            Animated.spring(dismissY, {
              toValue: 0,
              useNativeDriver: true,
            }).start();
          }
        }
      },
    });
  }, [zoom, dismissY, panX, panY, onClose, resetTransforms]);

  // ── Double-tap detection ──
  const handleImagePress = useCallback(() => {
    const now = Date.now();
    if (now - lastTapRef.current < 300) {
      handleDoubleTap();
      lastTapRef.current = 0;
    } else {
      lastTapRef.current = now;
    }
  }, [handleDoubleTap]);

  // ── Render ──
  const opacity = dismissY.interpolate({
    inputRange: [0, SCREEN_HEIGHT * 0.5],
    outputRange: [1, 0.3],
    extrapolate: 'clamp',
  });

  // #3945: Empty-images guard must come AFTER all hooks are registered.
  // Returning null here is now safe — every useState/useRef/useCallback/useMemo
  // above has already been called unconditionally in the same order regardless
  // of whether images is empty or non-empty.
  if (!images.length) {
    return null;
  }

  const current = images[safeIndex];
  const hasMultiple = images.length > 1;

  return (
    <Modal visible={visible} transparent animationType="fade" onRequestClose={onClose}>
      <View style={styles.overlay}>
        {/* Backdrop — tap to close */}
        <TouchableOpacity
          style={StyleSheet.absoluteFill}
          activeOpacity={1}
          onPress={onClose}
        />

        {/* Image container with gestures */}
        <Animated.View
          {...panResponder.panHandlers}
          testID="lightbox-image-container"
          style={[
            styles.imageContainer,
            {
              opacity,
              transform: [
                { translateY: zoom > MIN_ZOOM ? panY : dismissY },
                { translateX: zoom > MIN_ZOOM ? panX : 0 },
                { scale: zoom },
              ],
            },
          ]}
        >
          <TouchableOpacity activeOpacity={1} onPress={handleImagePress}>
            <RNImage
              source={{ uri: current.uri }}
              style={styles.image}
              resizeMode="contain"
              accessibilityLabel={current.alt || 'Image'}
              testID="lightbox-image"
            />
          </TouchableOpacity>
        </Animated.View>

        {/* Close button */}
        <TouchableOpacity
          style={styles.closeButton}
          onPress={onClose}
          testID="lightbox-close"
          accessibilityLabel="Close"
        >
          <Icon name="close" size={28} color="#fff" />
        </TouchableOpacity>

        {/* File name caption (#3927) — mirrors Obsidian 1.13.4 */}
        {imageFileName(current) ? (
          <View style={styles.fileNameBar} testID="lightbox-file-name">
            <Text
              style={styles.fileNameText}
              numberOfLines={1}
              ellipsizeMode="middle"
              testID="lightbox-file-name-text"
            >
              {imageFileName(current)}
            </Text>
          </View>
        ) : null}

        {/* Zoom controls (#3422, #3790) */}
        <View style={styles.zoomControls}>
          <TouchableOpacity
            style={styles.zoomButton}
            onPress={fitToScreen}
            testID="lightbox-fit"
            accessibilityLabel="Fit to screen"
          >
            <Icon name="fit-screen" size={24} color="#fff" />
          </TouchableOpacity>
          <TouchableOpacity
            style={styles.zoomButton}
            onPress={zoomOut}
            testID="lightbox-zoom-out"
            accessibilityLabel="Zoom out"
          >
            <Icon name="remove-circle" size={24} color="#fff" />
          </TouchableOpacity>
          <Text style={styles.zoomLabel} testID="lightbox-zoom-label">
            {zoomPercentage(zoom)}
          </Text>
          <TouchableOpacity
            style={styles.zoomButton}
            onPress={zoomIn}
            testID="lightbox-zoom-in"
            accessibilityLabel="Zoom in"
          >
            <Icon name="add-circle" size={24} color="#fff" />
          </TouchableOpacity>
        </View>

        {/* Navigation arrows (#3030) */}
        {hasMultiple && (
          <>
            <TouchableOpacity
              style={[styles.navButton, styles.navPrev]}
              onPress={() => navigate(-1)}
              testID="lightbox-prev"
              accessibilityLabel="Previous image"
            >
              <Icon name="arrow-back" size={28} color="#fff" />
            </TouchableOpacity>
            <TouchableOpacity
              style={[styles.navButton, styles.navNext]}
              onPress={() => navigate(1)}
              testID="lightbox-next"
              accessibilityLabel="Next image"
            >
              <Icon name="chevron-right" size={32} color="#fff" />
            </TouchableOpacity>

            {/* Position indicator */}
            <View style={styles.indicator}>
              <Text style={styles.indicatorText}>
                {safeIndex + 1} / {images.length}
              </Text>
            </View>
          </>
        )}
      </View>
    </Modal>
  );
}

const styles = StyleSheet.create({
  overlay: {
    flex: 1,
    backgroundColor: 'rgba(0, 0, 0, 0.92)',
    justifyContent: 'center',
    alignItems: 'center',
  },
  imageContainer: {
    width: SCREEN_WIDTH,
    height: SCREEN_HEIGHT * 0.8,
    justifyContent: 'center',
    alignItems: 'center',
  },
  image: {
    width: SCREEN_WIDTH,
    height: SCREEN_HEIGHT * 0.8,
  },
  closeButton: {
    position: 'absolute',
    top: Platform.OS === 'ios' ? 50 : 30,
    right: 20,
    width: 44,
    height: 44,
    borderRadius: 22,
    backgroundColor: 'rgba(0,0,0,0.5)',
    justifyContent: 'center',
    alignItems: 'center',
    zIndex: 10,
  },
  zoomControls: {
    position: 'absolute',
    bottom: 40,
    flexDirection: 'row',
    alignItems: 'center',
    gap: 12,
    backgroundColor: 'rgba(0,0,0,0.5)',
    borderRadius: 24,
    paddingHorizontal: 16,
    paddingVertical: 8,
    zIndex: 10,
  },
  zoomButton: {
    width: 36,
    height: 36,
    justifyContent: 'center',
    alignItems: 'center',
  },
  zoomLabel: {
    color: '#fff',
    fontSize: 14,
    fontWeight: '600',
    minWidth: 50,
    textAlign: 'center',
  },
  navButton: {
    position: 'absolute',
    top: '50%',
    marginTop: -22,
    width: 44,
    height: 44,
    borderRadius: 22,
    backgroundColor: 'rgba(0,0,0,0.5)',
    justifyContent: 'center',
    alignItems: 'center',
    zIndex: 10,
  },
  navPrev: { left: 12 },
  navNext: { right: 12 },
  indicator: {
    position: 'absolute',
    bottom: 100,
    backgroundColor: 'rgba(0,0,0,0.5)',
    borderRadius: 12,
    paddingHorizontal: 12,
    paddingVertical: 4,
  },
  indicatorText: {
    color: '#fff',
    fontSize: 13,
    fontWeight: '500',
  },
  // #3927: file name caption bar (top-center, mirrors Obsidian 1.13.4)
  fileNameBar: {
    position: 'absolute',
    top: Platform.OS === 'ios' ? 60 : 40,
    alignSelf: 'center',
    maxWidth: '80%',
    backgroundColor: 'rgba(0,0,0,0.55)',
    borderRadius: 12,
    paddingHorizontal: 12,
    paddingVertical: 4,
    zIndex: 10,
  },
  fileNameText: {
    color: '#fff',
    fontSize: 13,
    fontWeight: '500',
  },
});