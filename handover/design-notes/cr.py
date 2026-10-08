import sys,json,re,urllib.request
for doi in sys.argv[1:]:
    try:
        r=urllib.request.Request("https://api.crossref.org/works/"+doi,headers={'User-Agent':'x'})
        d=json.load(urllib.request.urlopen(r,timeout=60))['message']
        print(doi,d['title'],[a.get('family') for a in d.get('author',[])],d.get('container-title'),d.get('volume'),d.get('page'),d.get('issued'))
        print(re.sub('<[^>]+>','',d.get('abstract','NOABS')))
    except Exception as e: print(doi,'ERR',e)
