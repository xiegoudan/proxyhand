import { useEffect, useRef } from 'react'
import { EditorState, Compartment } from '@codemirror/state'
import { EditorView, keymap } from '@codemirror/view'
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
import { json, jsonParseLinter } from '@codemirror/lang-json'
import { linter, lintGutter, type Diagnostic } from '@codemirror/lint'
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language'
import { tags as t } from '@lezer/highlight'

/* Violet-tinted light theme so the editor matches the app. */
const editorTheme = EditorView.theme(
  {
    '&': { backgroundColor: 'transparent', color: 'hsl(240 12% 14%)' },
    '.cm-content': { caretColor: 'hsl(258 85% 50%)', padding: '8px 0' },
    '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'hsl(258 85% 50%)' },
    '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection': {
      backgroundColor: 'hsl(258 80% 62% / 0.22)',
    },
    '.cm-gutters': {
      backgroundColor: 'transparent',
      color: 'hsl(240 8% 62%)',
      border: 'none',
      paddingRight: '4px',
    },
    '.cm-activeLine': { backgroundColor: 'hsl(258 70% 96%)' },
    '.cm-activeLineGutter': { backgroundColor: 'transparent', color: 'hsl(258 70% 46%)' },
    '.cm-foldPlaceholder': {
      backgroundColor: 'hsl(258 70% 93%)',
      border: 'none',
      color: 'hsl(258 70% 44%)',
      padding: '0 4px',
    },
    '.cm-tooltip': {
      backgroundColor: 'hsl(0 0% 100%)',
      border: '1px solid hsl(240 10% 89%)',
      borderRadius: '6px',
      overflow: 'hidden',
    },
    '.cm-lintRange-error': {
      backgroundImage: 'none',
      textDecoration: 'underline wavy hsl(0 70% 46%)',
    },
  },
  { dark: false },
)

const jsonHighlight = HighlightStyle.define([
  { tag: t.propertyName, color: 'hsl(258 70% 46%)' },
  { tag: t.string, color: 'hsl(152 50% 30%)' },
  { tag: t.number, color: 'hsl(30 95% 36%)' },
  { tag: t.bool, color: 'hsl(350 72% 48%)' },
  { tag: t.null, color: 'hsl(350 72% 48%)' },
  { tag: t.punctuation, color: 'hsl(240 6% 46%)' },
])

const jsonLanguage = new Compartment()
const lintCompartment = new Compartment()

interface JsonEditorProps {
  value: string
  onChange?: (value: string) => void
  readOnly?: boolean
  minHeight?: number
  maxHeight?: number
  className?: string
}

/**
 * CodeMirror 6 wrapper for JSON editing: syntax highlighting, parse-error
 * linting, folding, bracket matching, history. Value stays in sync with the
 * prop without clobbering the user's cursor.
 */
export function JsonEditor({ value, onChange, readOnly = false, minHeight = 180, maxHeight = 420, className }: JsonEditorProps) {
  const hostRef = useRef<HTMLDivElement | null>(null)
  const viewRef = useRef<EditorView | null>(null)
  const onChangeRef = useRef(onChange)
  onChangeRef.current = onChange

  useEffect(() => {
    const host = hostRef.current
    if (!host) return
    const extensions = [
      editorTheme,
      syntaxHighlighting(jsonHighlight),
      jsonLanguage.of(json()),
      lintCompartment.of(readOnly ? [] : [linter(jsonParseLinter()), lintGutter()]),
      history(),
      keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
      EditorView.lineWrapping,
      EditorView.updateListener.of((update) => {
        if (update.docChanged && onChangeRef.current) {
          onChangeRef.current(update.state.doc.toString())
        }
      }),
      EditorState.readOnly.of(readOnly),
    ]
    const view = new EditorView({ state: EditorState.create({ doc: value, extensions }), parent: host })
    viewRef.current = view
    return () => {
      view.destroy()
      viewRef.current = null
    }
    // The editor is created once; value changes are applied below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // Sync external value changes into the editor (pretty/minify buttons).
  // The user has focus in here, so keep it and land the caret at the end
  // instead of letting the editor lose focus / collapse to a vanished cursor.
  useEffect(() => {
    const view = viewRef.current
    if (!view) return
    const current = view.state.doc.toString()
    if (value === current) return
    const hadFocus = view.hasFocus
    view.dispatch({
      changes: { from: 0, to: current.length, insert: value },
      selection: { anchor: value.length },
    })
    if (hadFocus) view.focus()
  }, [value])

  return (
    <div
      ref={hostRef}
      className={`cm-host ${className ?? ''}`}
      style={{ minHeight, maxHeight: maxHeight ? undefined : undefined, height: undefined }}
      data-min-height={minHeight}
      data-max-height={maxHeight}
    />
  )
}
