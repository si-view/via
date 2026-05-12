;;; via.il — SKILL bridge for the `via` IPC tool
;;;
;;; Connects Virtuoso to external processes via the `via` binary.
;;; In IC design a "via" punches through layers to carry a signal — here it
;;; carries SKILL expressions and JSON results between Virtuoso and the outside.
;;;
;;; Usage:
;;;   load("/path/to/via.il")
;;;   si_view_start("/usr/local/bin/via")
;;;   ; → CIW: [si-view] started  pid=…  bridge=…  sock=/tmp/via-<user>.sock
;;;
;;;   ; Third-party calls:
;;;   ;   via send --sock /tmp/via-<user>.sock --expr 'hiGetPoint()'
;;;
;;;   si_view_emit("progress" 75)
;;;   si_view_stop()
;;;
;;; IPC protocol
;;;   via serve stdout → Virtuoso:      SKILL_EXPRESSION\n
;;;   Virtuoso → via forward stdin:     S:JSON_VALUE\n  |  F:error\n  |  E:{…}\n
;;;   via forward → callback socket:    <TOKEN> S:JSON_VALUE\n  (etc.)
;;;
;;; Security
;;;   --cb-token  is generated fresh per session (local let-binding, never global).
;;;   It is passed through per-process temp files and deleted after startup.

;;; ── globals ───────────────────────────────────────────────────────────────────

_via_server_pid  = nil                     ; `via serve`   process PID
_via_bridge_pid  = nil                     ; `via forward` process PID
_via_ipc_buf     = ""                      ; line-reassembly buffer
_via_remote_tbl  = makeTable("_via_remote" nil) ; id → opaque object

;;; ── public API ────────────────────────────────────────────────────────────────

;;; Start `via serve` and `via forward` as Virtuoso child processes.
;;;
;;; binary_path  — absolute path to the `via` binary
;;; ?sock        — Unix socket path clients connect to
;;;                [default: /tmp/via-<USER>.sock]
procedure(si_view_start(binary_path
    @key
    (sock        sprintf(nil "/tmp/via-%s.sock" getShellEnvVar("USER")))
    (log_file    "via.log"))

  let((vpid cb_token cb_sock cb_token_file_serve cb_token_file_fwd
       serve_cmd serve_pid fwd_cmd bridge_pid port)

    vpid     = ipcGetPid()
    cb_token = _via_gen_token()      ; local — discarded when let-block ends
    cb_sock  = sprintf(nil "/tmp/via-cb-%d.sock" vpid)

    ;;; Write the token to two separate files — one per process — so each
    ;;; binary reads and deletes its own copy without any race condition.
    cb_token_file_serve = sprintf(nil "/tmp/via-token-serve-%d" vpid)
    cb_token_file_fwd   = sprintf(nil "/tmp/via-token-fwd-%d" vpid)

    port = outfile(cb_token_file_serve)
    fprintf(port "%s" cb_token)
    close(port)

    port = outfile(cb_token_file_fwd)
    fprintf(port "%s" cb_token)
    close(port)

    ;;; ── serve process ─────────────────────────────────────────────────────────
    ;;; --cb-token-file keeps the callback token out of ps aux.
    serve_cmd = sprintf(nil
      "%s serve --sock %s --cb-sock %s --cb-token-file %s --log-file %s"
      binary_path sock cb_sock cb_token_file_serve log_file)
    serve_pid = ipcBeginProcess(serve_cmd "" 'si_view_on_data "" t)
    if(!serve_pid then
      error("[si-view] failed to start serve: %s" binary_path)
    )
    _via_server_pid = serve_pid

    ;;; ── forward process ───────────────────────────────────────────────────────
    fwd_cmd = sprintf(nil
      "%s forward --cb-sock %s --cb-token-file %s"
      binary_path cb_sock cb_token_file_fwd)
    bridge_pid = ipcBeginProcess(fwd_cmd "" nil "" t)
    if(!bridge_pid then
      ipcKillProcess(_via_server_pid)
      _via_server_pid = nil
      error("[si-view] failed to start forward: %s" binary_path)
    )
    _via_bridge_pid = bridge_pid

    printf("\n  __   __ ___    _    \n  \\ \\ / /|_ _|  / \\   \n   \\ V /  | |  / _ \\  \n    \\_/  |___|/_/ \\_\\ \n  ================================\n   WeChat : si-view\n   GitHub : https://github.com/si-view/\n  ================================\n\n")
    printf("[si-view] started  pid=%d  bridge=%d  sock=%s\n"
      serve_pid bridge_pid sock)
    t
  )
)

;;; Stop both child processes.
procedure(si_view_stop()
  when(_via_server_pid
    ipcKillProcess(_via_server_pid)
    _via_server_pid = nil
  )
  when(_via_bridge_pid
    ipcKillProcess(_via_bridge_pid)
    _via_bridge_pid = nil
  )
  printf("[si-view] stopped\n")
)

;;; Push a named event with a SKILL value.
;;; The event is currently logged on the server side.
procedure(si_view_emit(name val)
  when(_via_bridge_pid
    ipcWriteProcess(_via_bridge_pid
      sprintf(nil "E:{\"name\":\"%s\",\"value\":%s}\n"
        _via_esc(name)
        _via_to_json(val)))
  )
)

;;; Retrieve an opaque remote object by its handle id.
;;;
;;; id  — the string id that appeared in an is_ref response, e.g.
;;;       the "id" field of {"id":"cellView:0x7f…","kind":"cellView"}
;;;
;;; Returns the live SKILL object, or nil if the id is not in the table.
;;;
;;; Example:
;;;   cv = via_remote("cellView:0x7f3a...")
procedure(via_remote(id)
  _via_remote_tbl[id]
)

;;; ── IPC callback ──────────────────────────────────────────────────────────────

;;; Called by Virtuoso when `via serve` writes a SKILL expression to stdout.
;;; Evaluates the expression and sends the result to the forwarder.
;;; Both evaluation and serialisation are wrapped in errset so that any SKILL
;;; error produces a well-formed F: response rather than a silent failure.
procedure(si_view_on_data(pid data)
  let((pos line result json_result errMsg reply)
    _via_ipc_buf = strcat(_via_ipc_buf data)
    pos = _via_find_nl(_via_ipc_buf)
    while(pos
      line         = substring(_via_ipc_buf 1 pos - 1)
      _via_ipc_buf = if(strlen(_via_ipc_buf) > pos then
                       substring(_via_ipc_buf pos + 1 strlen(_via_ipc_buf))
                     else
                       "")

      ;;; Step 1 — evaluate
      result = errset(evalstring(line) t)
      if(result then
        ;;; Step 2 — serialise (may fail for opaque objects; errset catches that)
        json_result = errset(_via_to_typed_json(car(result)) t)
        if(json_result then
          reply = strcat("S:" car(json_result) "\n")
        else
          reply = strcat("F:serialization error\n")
        )
      else
        errMsg = if(boundp('errText) && listp(errText)
                      && errText && stringp(car(errText)) then
                   car(errText)
                 else
                   "SKILL evaluation error")
        reply = strcat("F:" _via_esc(errMsg) "\n")
      )
      ipcWriteProcess(_via_bridge_pid reply)
      pos = _via_find_nl(_via_ipc_buf)
    )
  )
)

;;; ── internal helpers ──────────────────────────────────────────────────────────

;;; Generate a 32-character random hex token (local use only).
procedure(_via_gen_token()
  buildString(
    list(
      sprintf(nil "%04x" random(65536))
      sprintf(nil "%04x" random(65536))
      sprintf(nil "%04x" random(65536))
      sprintf(nil "%04x" random(65536))
      sprintf(nil "%04x" random(65536))
      sprintf(nil "%04x" random(65536))
      sprintf(nil "%04x" random(65536))
      sprintf(nil "%04x" random(65536)))
    "")
)

;;; 1-based index of first '\n' in str, or nil.
procedure(_via_find_nl(str)
  let((len i found)
    len   = strlen(str)
    i     = 1
    found = nil
    while(i <= len && !found
      if(substring(str i 1) == "\n" then
        found = i
      else
        i++
      )
    )
    found
  )
)

;;; ── SKILL → JSON ──────────────────────────────────────────────────────────────

;;; Return an EvalResult JSON string matching the Rust EvalResult struct:
;;;   {"data":<value>,"is_ref":false,"code":0}
;;;   {"data":{"id":"…","kind":"…"},"is_ref":true,"code":0}
;;;
;;; The SKILL type is carried implicitly by the JSON shape; `is_ref` flags
;;; remote/opaque objects so the caller knows `data` is a handle stub.
procedure(_via_to_typed_json(val)
  let((is_ref value_json)
    is_ref = nil
    cond(
      (null(val)
        value_json = "null")
      ((val == t)
        value_json = "true")
      (integerp(val)
        value_json = sprintf(nil "%d" val))
      (floatp(val)
        value_json = _via_fmt_float(val))
      (stringp(val)
        value_json = sprintf(nil "\"%s\"" _via_esc(val)))
      (symbolp(val)
        value_json = sprintf(nil "{\"__sym\":\"%s\"}" _via_esc(symbolToString(val))))
      (listp(val)
        value_json = _via_list_to_json(val))
      (tablep(val)
        value_json = _via_table_to_json(val))
      (t
        is_ref     = t
        let((id kind)
          id   = sprintf(nil "%A" val)
          kind = symbolToString(type(val))
          _via_remote_tbl[id] = val
          value_json = sprintf(nil "{\"id\":\"%s\",\"kind\":\"%s\"}"
                         _via_esc(id) kind)
        ))
    )
    sprintf(nil "{\"data\":%s,\"is_ref\":%s,\"code\":0}"
      value_json
      if(is_ref then "true" else "false"))
  )
)

procedure(_via_to_json(val)
  cond(
    (null(val)     "null")
    ((val == t)    "true")
    (integerp(val) sprintf(nil "%d" val))
    (floatp(val)   _via_fmt_float(val))
    (stringp(val)  sprintf(nil "\"%s\"" _via_esc(val)))
    (symbolp(val)  sprintf(nil "{\"__sym\":\"%s\"}" val))
    (listp(val)    _via_list_to_json(val))
    (tablep(val)   _via_table_to_json(val))
    (t             _via_remote_to_json(val))
  )
)

procedure(_via_fmt_float(v)
  let((s)
    s = sprintf(nil "%g" v)
    if(index(s ".") || index(s "e") || index(s "E") then
      s
    else
      strcat(s ".0"))
  )
)

;;; Escape a SKILL string for embedding in a JSON string literal.
procedure(_via_esc(s)
  let((i len ch out)
    out = ""
    len = strlen(s)
    for(i 1 len
      ch = substring(s i 1)
      cond(
        ((ch == "\\") out = strcat(out "\\\\"))
        ((ch == "\"") out = strcat(out "\\\""))
        ((ch == "\n") out = strcat(out "\\n"))
        ((ch == "\t") out = strcat(out "\\t"))
        (t            out = strcat(out ch))
      )
    )
    out
  )
)

procedure(_via_list_to_json(lst)
  sprintf(nil "[%s]" buildString(mapcar('_via_to_json lst) ","))
)

procedure(_via_table_to_json(tbl)
  let((pairs)
    pairs = list()
    foreach(key tbl
      pairs = append1(pairs
        sprintf(nil "\"%s\":%s"
          _via_esc(sprintf(nil "%s" key))
          _via_to_json(tbl[key])))
    )
    sprintf(nil "{%s}" buildString(pairs ","))
  )
)

;;; Store an opaque object (dbObject, cellView, etc.) in _via_remote_tbl and
;;; return a JSON stub for use inside _via_to_json (plain value context).
;;;
;;; The id is the %A representation of the object — human-readable and stable
;;; within a session.  To retrieve the object later use the public getter:
;;;   via_remote("<id>")
procedure(_via_remote_to_json(val)
  let((id kind)
    id   = sprintf(nil "%A" val)
    kind = symbolToString(type(val))
    _via_remote_tbl[id] = val
    sprintf(nil "{\"__remote\":\"%s\",\"__kind\":\"%s\"}"
      _via_esc(id) kind)
  )
)
