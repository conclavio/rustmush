import { mount } from 'svelte';
import Game from '../src/components/Game.svelte';
import { GameApp } from '../src/lib/game.svelte';
import '../src/app.css';
const app = new GameApp({url:'ws://127.0.0.1:1/fixture-disabled'});
app.session.status = 'connected';
app.session.character = {id:'#7',name:'Nicky'};
app.roomView = {id:'#1',name:'The Lantern',desc:[{text:'Local fixture. Commands: ask, text, queue, offline, online.'}],exits:[],contents:[]};
let id = 0;
let connected = true;
app.client.send = (command) => {
  if (!connected) return false;
  app.scrollback.pushText('ev-sys',`Sent: ${JSON.stringify(command)}`);
  return true;
};
function ask(kind: 'confirm'|'text') {
  app.requests.push({epoch:'fixture',id:++id,kind,prompt:[{text:kind==='text'?'Name this scene':'Seal the scene outcome?'}],options:[],deadline_ms:Date.now()+900000});
}
app.submit = (line:string) => {
  if (line === 'ask') ask('confirm');
  else if (line === 'text') ask('text');
  else if (line === 'queue') { ask('confirm'); ask('text'); }
  else if (line === 'offline') { connected = false; ask('text'); }
  else if (line === 'online') connected = true;
  else app.scrollback.pushText('ev-sys',`Command: ${line}`);
};
const tools = document.createElement('div');
tools.style.cssText='display:flex;gap:1rem;padding:0.5rem';
for (const [label,action] of [['Ask after 3s',()=>setTimeout(()=>ask('confirm'),3000)],['Go online',()=>{connected=true;}],['Queue asks',()=>{ask('confirm');ask('text');}],['Offline ask',()=>{connected=false;setTimeout(()=>{connected=true;},15000);ask('text');}]] as const) {
  const button = document.createElement('button');button.textContent=label;button.onclick=action;tools.append(button);
}
document.body.prepend(tools);
mount(Game,{target:document.getElementById('app')!,props:{app}});
