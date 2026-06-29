import type { NativeStackScreenProps } from '@react-navigation/native-stack';
import type { BottomTabScreenProps } from '@react-navigation/bottom-tabs';
import type { NavigatorScreenParams } from '@react-navigation/native';

export type ChatStackParamList = {
  ChatMain: { sessionId?: string; title?: string; prefillText?: string } | undefined;
  Sessions: undefined;
};

export type NotesStackParamList = {
  NotesList: undefined;
  NoteEdit: { noteId: string; blockId?: string | null };
};

export type RootTabParamList = {
  Chat: NavigatorScreenParams<ChatStackParamList>;
  Search: undefined;
  Notes: NavigatorScreenParams<NotesStackParamList>;
  Settings: undefined;
};

export type ChatScreenProps = NativeStackScreenProps<ChatStackParamList, 'ChatMain'>;
export type SessionsScreenProps = NativeStackScreenProps<ChatStackParamList, 'Sessions'>;
export type NotesScreenProps = NativeStackScreenProps<NotesStackParamList, 'NotesList'>;
export type NoteEditorScreenProps = NativeStackScreenProps<NotesStackParamList, 'NoteEdit'>;
export type SearchScreenProps = BottomTabScreenProps<RootTabParamList, 'Search'>;
