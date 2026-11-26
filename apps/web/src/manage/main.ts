import { mount } from "svelte";
import "../styles/base.css";
import "../flash/flash.css";
import Manage from "./Manage.svelte";

mount(Manage, { target: document.getElementById("app")! });
