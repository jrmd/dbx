"use client";
import * as React from "react";
import { useFrame, useThree } from "@react-three/fiber";
import * as THREE from "three";
export type AtmosphereKind = "silk" | "eclipse" | "tunnel" | "constellation";
const fragments: Record<AtmosphereKind, string> = {
  silk: `float v=0.; for(int i=0;i<7;i++){float f=float(i);float y=p.y+.19*sin(p.x*2.8+t*.3+f*.42)+.08*sin(p.x*6.-t*.2);v+=.006/(abs(y-f*.075+.24)+.007);} col=tint*v*.45;`,
  eclipse: `float r=length(p);float a=atan(p.y,p.x);float ring=exp(-abs(r-.49)*55.);float corona=exp(-abs(r-.5)*9.)*.18*(.6+.4*sin(a*9.+t*.25));col=tint*(ring+corona);col+=vec3(1.,.82,.5)*exp(-length(p-vec2(.35,.35))*23.);col*=smoothstep(.455,.48,r);`,
  tunnel: `float r=max(length(p),.015);float a=atan(p.y,p.x);float z=1./r+t*.4;float rings=pow(.5+.5*cos(z*5.),24.);float rays=pow(.5+.5*cos(a*14.+sin(z)*.3),34.);col=tint*(rings*.65+rays*.3)*smoothstep(.03,.35,r)*(.7+.3*sin(a+t*.15));`,
  constellation: `
float aa=2./resolution;
for(int i=0;i<38;i++){
float f=float(i);float y=1.-2.*(f+.5)/38.;float r=sqrt(1.-y*y);
float a=f*2.399963+t*.12;vec3 node=vec3(cos(a)*r,y,sin(a)*r);
vec2 q=(node.xy+vec2(node.z*.2,0.))*.78/(1.-node.z*.18);
float depth=.45+.55*(node.z*.5+.5);float d=length(p-q);
float sparkle=pow(.5+.5*sin(t*1.7+f*2.3),12.);
float pointSize=.004+depth*.003;
col+=tint*(1.-smoothstep(pointSize,pointSize+aa,d))*depth;
col+=tint*exp(-d*65.)*(.09+sparkle*.16)*depth;
if(sparkle>.5){vec2 star=abs(p-q);col+=tint*.25*sparkle*exp(-min(star.x,star.y)*550.-max(star.x,star.y)*85.);}
for(int j=0;j<38;j++){
if(j<=i)continue;float g=float(j);float y2=1.-2.*(g+.5)/38.;float r2=sqrt(1.-y2*y2);
float a2=g*2.399963+t*.12;vec3 node2=vec3(cos(a2)*r2,y2,sin(a2)*r2);
float span=length(node2-node);if(span>.68)continue;
vec2 q2=(node2.xy+vec2(node2.z*.2,0.))*.78/(1.-node2.z*.18);
vec2 ba=q2-q;float h=clamp(dot(p-q,ba)/max(dot(ba,ba),.00001),0.,1.);
float line=1.-smoothstep(0.,aa,length(p-q-ba*h));
float pulse=exp(-pow((h-fract(t*.23+f*.17+g*.11))*18.,2.));
col+=tint*line*(.12+pulse*.65)*depth;
}}
`,
};
export function AtmosphereScene({
  kind,
  color,
  speed,
}: {
  kind: AtmosphereKind;
  color: string;
  speed: number;
}) {
  const { size, invalidate } = useThree();
  const material = React.useMemo(
    () =>
      new THREE.ShaderMaterial({
        vertexShader: `varying vec2 uv0;void main(){uv0=uv;gl_Position=vec4(position.xy,0.,1.);}`,
        fragmentShader: `precision highp float; varying vec2 uv0; uniform float time; uniform float aspect; uniform float resolution; uniform vec3 tint; uniform vec2 pointer;void main(){vec2 p=(uv0-.5)*2.;p.x*=aspect;p-=pointer*.08;float t=time;vec3 col=vec3(.012,.016,.02);${fragments[kind]}gl_FragColor=vec4(col,1.);}`,
        uniforms: {
          time: { value: 0 },
          aspect: { value: 1 },
          resolution: { value: 500 },
          tint: { value: new THREE.Color(color) },
          pointer: { value: new THREE.Vector2() },
        },
      }),
    [kind, color],
  );
  React.useEffect(() => () => material.dispose(), [material]);
  React.useEffect(() => {
    material.uniforms.aspect.value = size.width / Math.max(size.height, 1);
    material.uniforms.resolution.value = Math.max(size.height, 1);
    invalidate();
  }, [material, size, invalidate]);
  useFrame(({ pointer }, delta) => {
    material.uniforms.time.value += Math.min(delta, 0.05) * speed;
    material.uniforms.pointer.value.lerp(pointer, 0.035);
  });
  return (
    <mesh material={material}>
      <planeGeometry args={[2, 2]} />
    </mesh>
  );
}
