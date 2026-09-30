export const screenVertex = `varying vec2 uvScreen; void main(){uvScreen=uv;gl_Position=vec4(position.xy,0.,1.);}`;
const common = `
varying vec2 uvScreen; uniform float time; uniform float aspect; uniform vec2 pointer; uniform vec3 tint;
float hash(vec3 p){p=fract(p*.3183099+vec3(.1,.2,.3));p*=17.;return fract(p.x*p.y*p.z*(p.x+p.y+p.z));}
float noise(vec3 p){vec3 i=floor(p),f=fract(p);f=f*f*(3.-2.*f);return mix(mix(mix(hash(i),hash(i+vec3(1,0,0)),f.x),mix(hash(i+vec3(0,1,0)),hash(i+vec3(1,1,0)),f.x),f.y),mix(mix(hash(i+vec3(0,0,1)),hash(i+vec3(1,0,1)),f.x),mix(hash(i+vec3(0,1,1)),hash(i+vec3(1,1,1)),f.x),f.y),f.z);}
float fbm(vec3 p){float v=0.,a=.5;for(int i=0;i<4;i++){v+=noise(p)*a;p=p*2.02+13.1;a*=.5;}return v;}
vec3 studio(vec3 r){float band=pow(max(0.,sin(r.y*5.+r.x*2.)),12.);vec3 c=mix(vec3(.035,.04,.085),vec3(.85,.90,1.),smoothstep(-.3,.9,r.y));c+=vec3(1.,.8,.6)*band*1.2;c+=vec3(.26,.38,.8)*pow(max(0.,r.x),6.);return c;}
`;
export const orbFragment =
  common +
  `
void main(){
vec2 q=(uvScreen-.5)*vec2(aspect,1.)*2.65-pointer*.04;
float radius=.88;float edge=length(q);float aa=fwidth(edge)*1.5;
vec3 bg=vec3(36.,28.,43.)/255.;
float z=sqrt(max(radius*radius-dot(q,q),0.));
vec3 n=normalize(vec3(q,z));
float turn=time*.12;vec3 r=vec3(n.x*cos(turn)+n.z*sin(turn),n.y,-n.x*sin(turn)+n.z*cos(turn));
// Broad moving studio ribbons give the sphere a reflective, pearlescent surface.
vec3 reflected=reflect(vec3(0.,0.,-1.),n);
float sweep=reflected.y*.85+reflected.x*.42+sin(reflected.x*2.8+turn)*.16;
float ribbon=exp(-pow((sweep-.28-sin(turn)*.12)*8.,2.));
float lower=exp(-pow((sweep+.58)*12.,2.));
float iridescence=.5+.5*sin(r.y*5.+r.x*3.+turn);
vec3 metal=mix(tint*.7,vec3(.28,.38,.76),iridescence);
float rim=pow(1.-max(n.z,0.),2.8);
vec3 c=metal*(.22+.6*max(n.y*.5+n.z*.7,0.));
c+=mix(vec3(.68,.76,1.),vec3(1.,.86,.73),iridescence)*ribbon*.95;
c+=tint*lower*.65+vec3(.48,.57,.92)*rim*.48;
float highlight=pow(max(dot(n,normalize(vec3(-.6,.8,1.2))),0.),45.);
c+=vec3(1.,.95,.9)*highlight*.45;
float coverage=1.-smoothstep(radius-aa,radius+aa,edge);
gl_FragColor=vec4(mix(bg,c,coverage),1.);}
`;
export const liquidFragment =
  common +
  `
float water(vec2 p){vec2 c=pointer*vec2(aspect,1.)*.6;float d=length(p-c);return sin(p.x*3.+p.y*2.+time*.45)*.15+sin(p.y*5.-p.x*1.4-time*.3)*.10+sin(d*10.-time*1.3)*exp(-d*2.)*.012;}
void main(){vec2 p=(uvScreen-.5)*vec2(aspect,1.)*3.;float e=.008;float dx=(water(p+vec2(e,0))-water(p-vec2(e,0)))/(2.*e);float dy=(water(p+vec2(0,e))-water(p-vec2(0,e)))/(2.*e);vec3 n=normalize(vec3(-dx,-dy,1.));vec3 r=reflect(normalize(vec3(p*.08,-1.)),n);float line=pow(.5+.5*sin((r.x+r.y)*9.+water(p)*8.),8.);vec3 c=mix(vec3(.055,.075,.17),tint*.65,clamp(r.y*.65+.45,0.,1.));c+=studio(r)*.4;c=mix(c,vec3(.94,.79,.60),line*.25);c*=1.-length(uvScreen-.5)*.3;gl_FragColor=vec4(c/(c+.65),1.);}
`;
export const distortionFragment =
  common +
  `
uniform sampler2D picture; uniform float imageAspect;
void main(){vec2 p=uvScreen;vec2 center=.5+pointer*.5;vec2 delta=(p-center)*vec2(aspect,1.);float d=length(delta);float lens=exp(-d*d*9.);vec2 offset=normalize(delta+vec2(.001))*sin(d*30.-time*1.1)*.018*lens;offset+=vec2(sin(p.y*7.+time*.3),cos(p.x*6.+time*.2))*.004;vec2 cover=vec2(min(1.,aspect/imageAspect),min(1.,imageAspect/aspect));vec2 sampleUv=(p-.5+offset)*cover+.5;float split=.008*lens;vec3 c=vec3(texture2D(picture,sampleUv+vec2(split,0)).r,texture2D(picture,sampleUv).g,texture2D(picture,sampleUv-vec2(split,0)).b);gl_FragColor=vec4(c,1.);}
`;
export const terrainVertex = `varying vec3 vPos;varying float elevation;uniform float time;
float hash(vec2 p){return fract(sin(dot(p,vec2(127.1,311.7)))*43758.5453);}float noise(vec2 p){vec2 i=floor(p),f=fract(p);f=f*f*(3.-2.*f);return mix(mix(hash(i),hash(i+vec2(1,0)),f.x),mix(hash(i+vec2(0,1)),hash(i+vec2(1,1)),f.x),f.y);}float hills(vec2 p){float v=0.,a=.5;for(int i=0;i<4;i++){v+=a*noise(p);p=p*2.03+3.1;a*=.5;}return v;}
void main(){vec3 p=position;float h=hills(p.xy*.6+vec2(time*.018,0));p.z=pow(h,2.)*2.-.5;elevation=p.z;vPos=p;gl_Position=projectionMatrix*modelViewMatrix*vec4(p,1.);}`;
export const terrainFragment =
  common +
  `
void main(){
vec2 p=(uvScreen-.5)*vec2(aspect,1.)*2.4+pointer*.06;
p+=vec2(time*.018,0.);
float h=sin(p.x*1.4+sin(p.y*1.6))*.35+cos(p.y*1.8-p.x*.3)*.3+sin(p.x*2.9+p.y*2.1)*.12;
float level=h*19.;float width=max(fwidth(level),.008);
float line=1.-smoothstep(width*.4,width*1.3,abs(fract(level)-.5));
float major=1.-smoothstep(width*.5,width*1.5,abs(fract(level/5.)-.5)*5.);
vec3 bg=vec3(20.,34.,30.)/255.;
vec3 c=mix(bg,tint,.035+line*.26+major*.25);
gl_FragColor=vec4(c,1.);}
`;
export const particleVertex = `attribute float seed;varying float brightness;uniform float time;uniform vec2 pointer;void main(){vec3 p=position;float a=time*.06+(length(p.xy))*.17;mat2 r=mat2(cos(a),-sin(a),sin(a),cos(a));p.xy=r*p.xy;p.z+=sin(seed*35.+time*.3)*.08;p.x+=pointer.x*.12;p.y+=pointer.y*.08;vec4 mv=modelViewMatrix*vec4(p,1.);brightness=.45+.55*seed;gl_PointSize=(1.2+seed*1.8)*(4./-mv.z);gl_Position=projectionMatrix*mv;}`;
export const particleFragment = `varying float brightness;uniform vec3 tint;void main(){float d=length(gl_PointCoord-.5);if(d>.5)discard;vec3 c=mix(tint,vec3(.92,.94,1.),brightness*.7);gl_FragColor=vec4(c,(1.-smoothstep(.12,.5,d))*brightness);}`;

/** Relief variant: tessellated terrain, cropped beyond the viewport edges. */
export const terrainReliefFragment = `varying vec3 vPos;varying float elevation;uniform vec3 tint;
void main(){vec3 n=normalize(cross(dFdx(vPos),dFdy(vPos)));
float light=.55+.45*abs(dot(n,normalize(vec3(-.5,.7,1.))));
float level=elevation*18.;float aa=max(fwidth(level),.006);
float ink=1.-smoothstep(aa*.4,aa*1.4,abs(fract(level)-.5));
vec3 c=mix(vec3(.07,.14,.13),tint,clamp(elevation*.65+.1,0.,.7));
c=mix(c,vec3(.78,.84,.58),ink*.46)*light;gl_FragColor=vec4(c,1.);}`;
