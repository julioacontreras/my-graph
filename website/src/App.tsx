import { AnnouncementBar } from "./components/AnnouncementBar";
import { Navbar } from "./components/Navbar";
import { Hero } from "./components/Hero";
import { WhyMyGraph } from "./components/WhyMyGraph";
import { Properties } from "./components/Properties";
import { GetStarted } from "./components/GetStarted/GetStarted";
import { Benchmarks } from "./components/Benchmarks/Benchmarks";
import { Architecture } from "./components/Architecture";
import { UseCases } from "./components/UseCases";
import { Limitations } from "./components/Limitations";
import { ForAgents } from "./components/ForAgents";
import { Community } from "./components/Community";
import { FinalCta } from "./components/FinalCta";
import { Footer } from "./components/Footer";

export function App() {
  return (
    <>
      <a href="#main" className="sr-only focus:not-sr-only focus:fixed focus:left-4 focus:top-4 focus:z-50 focus:rounded-md focus:bg-primary focus:px-4 focus:py-2 focus:text-white">
        Skip to content
      </a>
      <AnnouncementBar />
      <Navbar />
      <main id="main">
        <Hero />
        <WhyMyGraph />
        <Properties />
        <GetStarted />
        <Benchmarks />
        <Architecture />
        <UseCases />
        <Limitations />
        <ForAgents />
        <Community />
        <FinalCta />
      </main>
      <Footer />
    </>
  );
}
