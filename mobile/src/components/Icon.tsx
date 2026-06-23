/**
 * Centralized icon component — replaces all text emoji with Ionicons.
 * Usage: <Icon name="camera" size={18} color="#fff" />
 */
import React from 'react';
import Ionicons from '@expo/vector-icons/Ionicons';

export type IconName =
  | 'menu' | 'plus' | 'close' | 'search' | 'send' | 'stop'
  | 'camera' | 'image' | 'document' | 'mic' | 'mic-off'
  | 'smile' | 'eye' | 'eye-off' | 'edit' | 'pin' | 'link'
  | 'rocket' | 'wifi-off' | 'trash' | 'copy' | 'check'
  | 'error' | 'star' | 'clipboard' | 'globe' | 'pencil'
  | 'sun' | 'moon' | 'refresh' | 'export' | 'import'
  | 'chevron-right' | 'chevron-down' | 'arrow-back'
  | 'settings' | 'chatbubble' | 'add-circle';

const ICON_MAP: Record<IconName, keyof typeof Ionicons.glyphMap> = {
  'menu': 'menu',
  'plus': 'add',
  'close': 'close',
  'search': 'search',
  'send': 'arrow-up',
  'stop': 'stop',
  'camera': 'camera',
  'image': 'image',
  'document': 'document-text',
  'mic': 'mic',
  'mic-off': 'mic-off',
  'smile': 'happy-outline',
  'eye': 'eye',
  'eye-off': 'eye-off',
  'edit': 'create',
  'pin': 'pin',
  'link': 'link',
  'rocket': 'rocket',
  'wifi-off': 'cloud-offline',
  'trash': 'trash',
  'copy': 'copy',
  'check': 'checkmark-circle',
  'error': 'close-circle',
  'star': 'star',
  'clipboard': 'clipboard',
  'globe': 'globe',
  'pencil': 'pencil',
  'sun': 'sunny',
  'moon': 'moon',
  'refresh': 'refresh',
  'export': 'share-outline',
  'import': 'download-outline',
  'chevron-right': 'chevron-forward',
  'chevron-down': 'chevron-down',
  'arrow-back': 'arrow-back',
  'settings': 'settings-outline',
  'chatbubble': 'chatbubble-outline',
  'add-circle': 'add-circle-outline',
};

interface IconProps {
  name: IconName;
  size?: number;
  color?: string;
  style?: any;
}

export default function Icon({ name, size = 18, color = '#000', style }: IconProps) {
  const glyph = ICON_MAP[name];
  if (!glyph) return null;
  return <Ionicons name={glyph} size={size} color={color} style={style} />;
}
