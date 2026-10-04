#!/bin/bash
# usage: xmatrix2.sh <binary> <outdir> <mode: math|tool>
BIN=$1; OUT=$2; MODE=$3; mkdir -p $OUT
export TMPDIR=/private/tmp
MODELS="google:gemini-3.8-flash dddai:claude-fable-5-1 zai:glm-5.3 xai:grok-4.3"
if [ "$MODE" = tool ]; then
  P1="Use the read tool to read notes.txt, then tell me the number written in it. Think briefly. Do not edit anything."
  P2="Add 1 to that number and reply with just the result."
else
  P1="What is 17 times 23? Think it through briefly, then answer with just the number."
  P2="Add 1 to your previous answer. Reply with just the number."
fi
pair() {
  local p=$1 c=$2; local pp=${p%%:*} pm=${p#*:} cp=${c%%:*} cm=${c#*:}
  local dir=$OUT/$pp-to-$cp; rm -rf $dir; mkdir -p $dir; cd $dir; git init -q -b main; echo "41" > notes.txt
  $BIN -p "$P1" --provider $pp --model $pm --approval allow-all --output-format json > t1.json 2> t1.err
  local sid=$(python3 -c "import json; print(json.load(open('t1.json')).get('session_id',''))" 2>/dev/null)
  local s1=$(python3 -c "import json; d=json.load(open('t1.json')); print(d.get('status'), repr((d.get('output') or '')[:12]))" 2>/dev/null)
  $BIN -p "$P2" --resume $sid --provider $cp --model $cm --approval allow-all --output-format json > t2.json 2> t2.err
  local s2=$(python3 -c "import json; d=json.load(open('t2.json')); print(d.get('status'), '|', (d.get('output') or '')[:24].replace(chr(10),' '), '|', (d.get('error') or '')[:100])" 2>/dev/null || tail -c 160 t2.err | tr '\n' ' ')
  echo "$pp -> $cp : t1=$s1 t2=$s2"
}
for p in $MODELS; do for c in $MODELS; do [ "$p" = "$c" ] && continue; pair $p $c & done; wait; done
