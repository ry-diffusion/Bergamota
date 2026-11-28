/* From Jesvi iOS / HakiOS (S20 port), userland/src/brightness_policy.h, MIT License, Copyright (c) 2025-2026 the HakiOS authors.
 * Unchanged except the include guard. The nit figures (3..420, the S20 OLED's range) are NOT calibrated for the Poco F1's
 * panel (docs/SENSORS.md). */
/* Adaptive ambient brightness policy. Original implementation informed by
 * AOSP AutomaticBrightnessController's hysteresis/debounce/short-term-model
 * design; no Android source copied. Times are monotonic seconds, nits nominal.
 *
 * Timing (A, 2026-11-13, user: "not responsive or quick"): the light sensor reports every 250 ms (kernel
 * shub_light_ms). A new level must hold for 1.5 s before brightening or 3 s before dimming (was 3/5 s); a big change
 * (4x the accepted level or more, e.g. into sunlight or a dark room) after 0.5 s / 1.5 s. Wake holds 1 s (was 5). */
#ifndef BERGAMOTA_BRIGHTNESS_POLICY_H
#define BERGAMOTA_BRIGHTNESS_POLICY_H
#include <math.h>
#include <stdbool.h>
struct rb_policy {
 double accepted, candidate, mean, candidate_at, hold_until, target;
 double anchor_lux, anchor_nits;
 unsigned samples;
 bool ready, anchored;
};
/* The Control Center slider's legacy backlight value (0..1, as backboardd writes it: 0.025 + 0.95 s) to the nits the
 * kernel shows for it (MiniIOMobileFramebuffer.cpp legacyToNits16, fb_bl_curve=1): 2 + 418 s^2.2. */
static double rb_slider_nits(double value)
{
 double s=(value-.025)/.95;
 s=s<0?0:s>1?1:s;
 return 2+418*pow(s,2.2);
}
static double rb_base(double lux)
{
 static const double ambient[] = {0,3,10,150,1500}, nits[] = {3,8,30,150,420};
 if(lux<=0) return 3;
 for(unsigned i=1;i<5;i++) if(lux<=ambient[i]) {
  double t=i<=2?(lux-ambient[i-1])/(ambient[i]-ambient[i-1]):
   (log1p(lux)-log1p(ambient[i-1]))/(log1p(ambient[i])-log1p(ambient[i-1]));
  return nits[i-1]+t*(nits[i]-nits[i-1]);
 }
 return 420;
}
static double rb_target(const struct rb_policy *p,double lux)
{
 double baseline=rb_base(lux);
 if(!p->anchored) return baseline;
 /* Hold the chosen level in similar lighting (within 1.5x, or 3 lux in the dark). Across a larger change, blend
  * back toward the ordinary curve (fully by 4.5x) rather than carrying an extreme multiplier into every environment.
  * (Was 2x or 20 lux: a level chosen at 30 lux stayed in force at 12 lux, so the dark room never dimmed.) */
 double distance=fabs(log1p(lux)-log1p(p->anchor_lux));
 double absolute=fabs(lux-p->anchor_lux);
 if(distance<=log(1.5) || absolute<=3) return p->anchor_nits;
 double blend=fmax(0,fmin(1,(distance-log(1.5))/log(3.)));
 double result=exp(log(p->anchor_nits)*(1-blend)+log(baseline)*blend);
 return lux>p->anchor_lux?fmax(p->anchor_nits,result):fmin(p->anchor_nits,result);
}
static void rb_wake(struct rb_policy *p,double now,double nits)
{
 p->candidate=-1;p->samples=0;p->candidate_at=now;
 p->hold_until=now+1;p->target=fmax(3,fmin(420,nits));
}
static void rb_manual(struct rb_policy *p,double now,double lux,double nits)
{
 p->anchor_lux=fmax(0,lux);p->anchor_nits=fmax(3,fmin(420,nits));p->anchored=true;
 p->accepted=p->anchor_lux;p->ready=true;
 rb_wake(p,now,p->anchor_nits);
}
static double rb_observe(struct rb_policy *p,double now,double lux)
{
 if(!isfinite(lux)||lux<0) return p->target;
 double tolerance=fmax(2,.25*(p->candidate+1));
 if(p->candidate<0||fabs(lux-p->candidate)>tolerance) {
  p->candidate=lux;p->mean=lux;p->candidate_at=now;p->samples=1;
 } else {
  if(p->samples<32)p->samples++;
  p->mean+=(lux-p->mean)/p->samples;
 }
 bool up=!p->ready||p->mean>p->accepted;
 double ratio=(p->mean+1)/(p->accepted+1);
 bool big=p->ready&&(ratio>=4||ratio<=.25);
 double delay=up?(big?.5:1.5):(big?1.5:3);
 if(now>=p->hold_until&&now-p->candidate_at>=delay) {
  double threshold=fmax(3,.2*(p->accepted+1));
  if(!p->ready||fabs(p->mean-p->accepted)>threshold) {
   p->accepted=p->mean;p->ready=true;
  }
  if(p->ready) p->target=fmax(3,fmin(420,rb_target(p,p->accepted)));
 }
 return p->target;
}
#endif
