import type { NativeStackScreenProps } from '@react-navigation/native-stack';
import type { NavigatorScreenParams } from '@react-navigation/native';

export type ChatStackParamList = {
  ChatMain: { sessionId?: string; title?: string; prefillText?: string } | undefined;
  Sessions: undefined;
};

export type NotesStackParamList = {
  NotesList: undefined;
  NoteEdit: { noteId: string };
};

export type RootTabParamList = {
  Chat: NavigatorScreenParams<ChatStackParamList>;
  Notes: NavigatorScreenParams<NotesStackParamList>;
  Settings: undefined;
};

export type ChatScreenProps = NativeStackScreenProps<ChatStackParamList, 'ChatMain'>;
export type SessionsScreenProps = NativeStackScreenProps<ChatStackParamList, 'Sessions'>;
export type NotesScreenProps = NativeStackScreenProps<NotesStackParamList, 'NotesList'>;
export type NoteEditorScreenProps = NativeStackScreenProps<NotesStackParamList, 'NoteEdit'>;
