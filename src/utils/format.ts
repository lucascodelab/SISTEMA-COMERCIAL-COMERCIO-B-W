export const money=(cents:number)=>new Intl.NumberFormat("pt-BR",{style:"currency",currency:"BRL"}).format(cents/100);
export const date=(value?:string)=>value?new Intl.DateTimeFormat("pt-BR",{dateStyle:"short",timeStyle:"short"}).format(new Date(value.replace(" ","T"))):"—";
export const parseMoney=(value:string)=>Math.round(Number(value.replace(".","").replace(",","."))*100)||0;
