import type { NativeStackScreenProps } from '@react-navigation/native-stack';

export type ChatStackParamList = {
  ChatMain: { sessionId?: string; title?: string } | undefined;
  Sessions: undefined;
};

export type NotesStackParamList = {
  NotesList: undefined;
  NoteEdit: { noteId: string };
};

export type ChatScreenProps = NativeStackScreenProps<ChatStackParamList, 'ChatMain'>;
export type SessionsScreenProps = NativeStackScreenProps<ChatStackParamList, 'Sessions'>;
export type NotesScreenProps = NativeStackScreenProps<NotesStackParamList, 'NotesList'>;
export type NoteEditorScreenProps = NativeStackScreenProps<NotesStackParamList, 'NoteEdit'>;
