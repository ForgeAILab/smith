#!/bin/bash
# usage: xpair.sh <binary> <outdir> <producer> <consumer>
BIN=$1; OUT=$2; p=$3; c=$4; export TMPDIR=/private/tmp
pp=${p%%:*}; pm=${p#*:}; cp=${c%%:*}; cm=${c#*:}
dir=$OUT/$pp-to-$cp; rm -rf $dir; mkdir -p $dir; cd $dir; git init -q -b main; echo "41" > notes.txt
$BIN -p "Use the read tool to read notes.txt, then tell me the number written in it. Think briefly. Do not edit anything." --provider $pp --model $pm --approval allow-all --output-format json > t1.json 2> t1.err
sid=$(python3 -c "import json; print(json.load(open('t1.json')).get('session_id',''))" 2>/dev/null)
$BIN -p "Add 1 to that number and reply with just the result." --resume $sid --provider $cp --model $cm --approval allow-all --output-format json > t2.json 2> t2.err
echo "$pp -> $cp : t1=$(python3 -c "import json; d=json.load(open('t1.json')); print(d.get('status'))") t2=$(python3 -c "import json; d=json.load(open('t2.json')); print(d.get('status'), '|', (d.get('output') or '')[:20].replace(chr(10),' '), '|', (d.get('error') or '')[:120])")"
