import re,json
p='tools/evidence/cinematic-impact.json'
s=open(p).read()
s=re.sub(r'<<<<<<< [^\n]*\n(.*?)=======\n(.*?)>>>>>>> [^\n]*\n',lambda m:m.group(1)+"    },\n    {\n"+m.group(2),s,flags=re.S)
json.loads(s)
open(p,'w').write(s)
